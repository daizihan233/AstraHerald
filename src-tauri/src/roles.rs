//! 好友身份（`Config::roles`）的自动匹配。
//!
//! 规则：拿好友的备注名去撞课表里的课程名（`schedule.rs` 的 /subjects）。
//! 撞上就把这门课的名字写成他的身份，撞不上落一个通用身份「课程」。
//! 身份目前只有一个消费方：文件消息里的课件要不要自动打开。

use std::collections::HashMap;

use crate::notice::bot_handle;
use crate::schedule::subject_names;
use crate::utils::friend_info::{Friend, friend_list, label};
use serde::Serialize;

/// 匹配不上课程名时的兜底身份。
pub const FALLBACK_ROLE: &str = "课程";

/// 备注名里出现课程名，就算这位老师教这门课。
///
/// 同时撞上多门时，比的是**撞上的那个名字**有多长，不是课程全名有多长：
/// 课表里「课」→「课后辅导」这种单字简称要是按全名长度比，就会在「物理课代表」
/// 里压过「物理」（4 > 2）。同长时优先全名命中。
pub fn match_role(name: &str, subjects: &[(String, String)]) -> String {
    // (命中的课程, 命中名字的长度, 命中的是不是全名)
    let mut best: Option<(&(String, String), usize, bool)> = None;
    for subject in subjects {
        let mut hit: Option<(usize, bool)> = None;
        for (needle, is_full) in [(subject.0.as_str(), false), (subject.1.as_str(), true)] {
            if needle.is_empty() || !name.contains(needle) {
                continue;
            }
            let score = needle.chars().count();
            let better = match hit {
                None => true,
                Some((len, was_full)) => score > len || (score == len && is_full && !was_full),
            };
            if better {
                hit = Some((score, is_full));
            }
        }
        if let Some((score, is_full)) = hit {
            let better = match best {
                None => true,
                Some((_, len, was_full)) => score > len || (score == len && is_full && !was_full),
            };
            if better {
                best = Some((subject, score, is_full));
            }
        }
    }

    match best {
        // 身份存全名；课程只有简称时用简称，总比留空强。
        Some(((_, full), _, _)) if !full.is_empty() => full.clone(),
        Some(((short, _), _, _)) => short.clone(),
        None => FALLBACK_ROLE.to_string(),
    }
}

/// 拿来做匹配的名字：备注优先，没有备注就用昵称。
fn display_name(friend: &Friend) -> &str {
    friend
        .remark
        .as_deref()
        .or(friend.nickname.as_deref())
        .unwrap_or("")
}

fn roles_for(friends: &[Friend], subjects: &[(String, String)]) -> HashMap<String, String> {
    friends
        .iter()
        .map(|friend| {
            (
                friend.qq.to_string(),
                match_role(display_name(friend), subjects),
            )
        })
        .collect()
}

/// 「一键配置」：把好友列表整体过一遍，逐个判定身份。
///
/// 课表服务连不上时科目表是空的，这时所有人都是「课程」——仍然是有用的
/// 结果（至少把好友列表填进来了），所以不因此报错。
#[tauri::command]
pub async fn auto_fill_roles() -> Result<HashMap<String, String>, String> {
    let bot = bot_handle().ok_or_else(|| "bot 还没就绪".to_string())?;
    let friends = friend_list(&bot).await;
    if friends.is_empty() {
        return Err("还没拿到好友列表，等 bot 连上后再试".to_string());
    }

    // /subjects 是同步 TCP 调用，挪到阻塞线程池上，别占着 tokio 的工作线程。
    let subjects = tauri::async_runtime::spawn_blocking(subject_names)
        .await
        .unwrap_or_default();
    Ok(roles_for(&friends, &subjects))
}

/// 「好友身份」下拉里的一项：value 才是写进 config.json 的东西（QQ 号），
/// label 只是给人看的「备注 [用户名] （QQ号）」。
#[derive(Serialize)]
pub struct FriendOption {
    pub value: String,
    pub label: String,
}

/// 给「好友身份」卡片的下拉用：全量好友，一人一项，按 QQ 号升序。
///
/// 身份本来就只能从好友里挑，所以界面不必让人手打号码。
#[tauri::command]
pub async fn friend_options() -> Result<Vec<FriendOption>, String> {
    let bot = bot_handle().ok_or_else(|| "bot 还没就绪".to_string())?;
    let friends = friend_list(&bot).await;
    if friends.is_empty() {
        return Err("还没拿到好友列表，等 bot 连上后再试".to_string());
    }
    Ok(friends
        .iter()
        .map(|friend| {
            let qq = friend.qq.to_string();
            FriendOption {
                label: label(&qq, friend.remark.as_deref(), friend.nickname.as_deref()),
                value: qq,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friend_label_keeps_remark_and_nickname() {
        assert_eq!(
            label("10001", Some("数学老师"), Some("小王")),
            "数学老师 [小王] （10001）"
        );
        assert_eq!(label("10001", None, Some("小王")), "[小王] （10001）");
        assert_eq!(label("10001", Some("数学老师"), None), "数学老师 （10001）");
        assert_eq!(label("10001", None, None), "（10001）");
    }

    fn subjects() -> Vec<(String, String)> {
        vec![
            ("语".to_string(), "语文".to_string()),
            ("数".to_string(), "数学".to_string()),
            ("高数".to_string(), "高等数学".to_string()),
        ]
    }

    #[test]
    fn remark_matches_subject_name() {
        assert_eq!(match_role("语文老师", &subjects()), "语文");
        assert_eq!(match_role("数学组 王老师", &subjects()), "数学");
        // 全名和简称都能撞上。
        assert_eq!(match_role("数老师", &subjects()), "数学");
    }

    /// 同时撞上多门时取更长的那门：「高数老师」不该被判成「数学」。
    #[test]
    fn longest_subject_wins() {
        assert_eq!(match_role("高数老师", &subjects()), "高等数学");
    }

    /// 匹配不上就是兜底身份；空课程名不能把所有人都匹配上。
    #[test]
    fn unmatched_falls_back_to_course() {
        assert_eq!(match_role("班主任", &subjects()), FALLBACK_ROLE);
        assert_eq!(match_role("张三", &[]), FALLBACK_ROLE);
        assert_eq!(
            match_role("张三", &[(String::new(), String::new())]),
            FALLBACK_ROLE
        );
    }

    /// 只有简称的课程存简称，不能存成空身份。
    #[test]
    fn short_only_subject_uses_short_name() {
        let only_short = vec![("化".to_string(), String::new())];
        assert_eq!(match_role("化学老师", &only_short), "化");
    }

    /// 真实课表里「课」= 课后辅导，单字简称不能压过更具体的那门课。
    #[test]
    fn real_catalog_single_char_short_name_does_not_steal_the_match() {
        let real = vec![
            ("课".to_string(), "课后辅导".to_string()),
            ("物".to_string(), "物理".to_string()),
            ("考物".to_string(), "物理考试".to_string()),
        ];
        assert_eq!(match_role("物理课代表", &real), "物理");
        assert_eq!(match_role("课后辅导群", &real), "课后辅导");
        assert_eq!(match_role("物理考试出题人", &real), "物理考试");
    }

    #[test]    fn roles_for_covers_every_friend() {
        let friends = vec![
            Friend {
                qq: 2,
                remark: Some("语文老师".to_string()),
                nickname: None,
            },
            Friend {
                qq: 1,
                remark: None,
                nickname: Some("班主任".to_string()),
            },
        ];
        let roles = roles_for(&friends, &subjects());
        assert_eq!(roles["2"], "语文");
        assert_eq!(roles["1"], FALLBACK_ROLE);
        assert_eq!(roles.len(), 2);
    }
}
