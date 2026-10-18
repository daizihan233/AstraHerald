//! 好友/备注信息：把 QQ 号格式化成「备注 [用户名] （QQ号）」。
//!
//! 备注只能从 get_friend_list 拿（私聊事件的 sender 里压根没有 remark 字段），
//! 所以这里维护一份带 TTL 的名单缓存，避免每条消息都去拉一次全量好友列表。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use kovi::RuntimeBot;
use kovi::event::id::ID;
use kovi_onebot::OnebotTrait;
use serde_json::Value;

/// 取非空字符串字段：字段缺失或为空串都返回 None。
fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// 名称缓存有效期。好友变动不频繁，5 分钟足够，还能挡住「每条消息拉一次全量好友列表」。
const NAME_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
struct CachedName {
    remark: Option<String>,
    nickname: Option<String>,
    at: Instant,
    /// 是否来自好友列表。陌生人接口也往这张表里放昵称，
    /// 「一键配置好友身份」只该看到真好友，所以这个标记不能省。
    friend: bool,
}

#[derive(Default)]
struct NameCache {
    /// 上次全量拉好友列表的时间；决定「缓存未命中」时要不要重拉列表。
    list_at: Option<Instant>,
    /// 好友（remark 有值）和陌生人（remark 为 None）共用一张表。
    names: HashMap<i64, CachedName>,
}

static NAME_CACHE: OnceLock<Mutex<NameCache>> = OnceLock::new();

/// 锁只在校验/写入时短暂持有，绝不跨 await。
fn cache() -> MutexGuard<'static, NameCache> {
    NAME_CACHE
        .get_or_init(|| Mutex::new(NameCache::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

impl NameCache {
    fn get(&self, qq: i64) -> Option<(Option<String>, Option<String>)> {
        let entry = self.names.get(&qq)?;
        (entry.at.elapsed() < NAME_TTL).then(|| (entry.remark.clone(), entry.nickname.clone()))
    }
}

fn cached_name(qq: i64) -> Option<(Option<String>, Option<String>)> {
    cache().get(qq)
}

fn list_stale() -> bool {
    let c = cache();
    c.list_at.map_or(true, |t| t.elapsed() >= NAME_TTL)
}

/// 拉一次全量好友列表刷新缓存；失败返回 false（保持陈旧，下条消息再试）。
async fn refresh_friends(bot: &RuntimeBot) -> bool {
    let Ok(ret) = bot.get_friend_list().await else {
        return false;
    };
    let Some(friends) = ret.data.as_array() else {
        return false;
    };
    let now = Instant::now();
    let mut c = cache();
    // 顺手清理过期条目，防止陌生人条目无限增长
    c.names.retain(|_, e| e.at.elapsed() < NAME_TTL);
    c.list_at = Some(now);
    for f in friends {
        if let Some(qq) = f["user_id"].as_i64() {
            c.names.insert(
                qq,
                CachedName {
                    remark: str_field(f, "remark"),
                    nickname: str_field(f, "nickname"),
                    at: now,
                    friend: true,
                },
            );
        }
    }
    true
}

/// 先查缓存；未命中才决定要不要重拉好友列表，最后回退陌生人接口。结果写回缓存。
async fn name_of(bot: &RuntimeBot, qq: i64) -> (Option<String>, Option<String>) {
    if let Some(hit) = cached_name(qq) {
        return hit;
    }
    if list_stale() && refresh_friends(bot).await {
        if let Some(hit) = cached_name(qq) {
            return hit;
        }
    }
    let nickname = bot
        .get_stranger_info(qq, false)
        .await
        .ok()
        .and_then(|r| str_field(&r.data, "nickname"));
    if nickname.is_some() {
        // 非好友（群临时会话）：remark 为 None；昵称也缓存，免得每条消息都问一次
        cache().names.insert(
            qq,
            CachedName {
                remark: None,
                nickname: nickname.clone(),
                at: Instant::now(),
                friend: false,
            },
        );
    }
    (None, nickname)
}

/// 一位好友：匹配课程名用的是备注（没有备注就退而用昵称）。
pub struct Friend {
    pub qq: i64,
    pub remark: Option<String>,
    pub nickname: Option<String>,
}

/// 全量好友（不含只在陌生人接口里出现过的号码），按 QQ 号升序。
///
/// 「一键配置好友身份」用它：缓存新鲜就直接用缓存，陈旧才真去拉一次
/// get_friend_list，免得点一下按钮就多打一次接口。
pub async fn friend_list(bot: &RuntimeBot) -> Vec<Friend> {
    if list_stale() {
        refresh_friends(bot).await;
    }
    let c = cache();
    let mut friends: Vec<Friend> = c
        .names
        .iter()
        .filter(|(_, entry)| entry.friend)
        .map(|(qq, entry)| Friend {
            qq: *qq,
            remark: entry.remark.clone(),
            nickname: entry.nickname.clone(),
        })
        .collect();
    friends.sort_by_key(|friend| friend.qq);
    friends
}

/// 给定 QQ 号，返回「备注 [用户名] （QQ号）」。
///
/// 备注只能从 get_friend_list 拿；非好友（如群临时会话）查不到备注，
/// 退化为「[用户名] （QQ号）」；用户名也拿不到时只剩「（QQ号）」。
/// 好友列表按 NAME_TTL 缓存，不会每条消息都去拉一次。
pub async fn get_friend_info(bot: Arc<RuntimeBot>, id: ID) -> String {
    // ID::try_as_i64() 直接给 Option<i64>（kovi id.rs:24），解析不出就只剩号码本身
    let Some(qq) = id.try_as_i64() else {
        return format!("（{id}）");
    };

    let (remark, nickname) = name_of(&bot, qq).await;
    label(&id.to_string(), remark.as_deref(), nickname.as_deref())
}

/// 好友在界面上的标识：「备注 [用户名] （QQ号）」，缺哪段就省哪段。
///
/// 提醒窗口和「好友身份」的下拉共用这一套写法，免得同一个好友在两处长得不一样。
pub fn label(qq: &str, remark: Option<&str>, nickname: Option<&str>) -> String {
    match (remark, nickname) {
        (Some(remark), Some(nickname)) => format!("{remark} [{nickname}] （{qq}）"),
        (None, Some(nickname)) => format!("[{nickname}] （{qq}）"),
        (Some(remark), None) => format!("{remark} （{qq}）"),
        (None, None) => format!("（{qq}）"),
    }
}
