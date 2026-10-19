use kovi::PluginBuilder as plugin;
use kovi::config::kovi_conf::KoviConf;
use kovi::event::MessageEventTrait;
use kovi::event::id::ID;
use kovi_onebot::*;

use crate::config::{Config, SceneAction, CONFIG_PATH};
use crate::notice::ReplyTarget;
use crate::utils::{find_matched_keywords, get_friend_info, need_to_solve, set_access_control};

/// 插件主体，被 `src/lib.rs` 根部的 `#[kovi::plugin]` 调用（属性宏只能贴 crate 根部，
/// 原因见 lib.rs 的注释）。
pub(crate) async fn plugin_main() {
    // `get_runtime_bot()` 读的是 kovi 的 task-local `PLUGIN_BUILDER`，它只在插件 main()
    // 运行期间有效（kovi-0.13.5/src/plugin.rs:75-90 的 `PLUGIN_BUILDER.scope`）。消息处理
    // 函数是 bot 事件循环里的**另一个 task**，在里面再调这些函数就会 panic
    // "Using PluginBuilder in wrong place"（plugin_builder.rs:115）——所以这里先把 bot
    // 抓出来，每条消息 clone 一份进闭包。
    let bot = plugin::get_runtime_bot();
    set_access_control(bot.clone());
    // 提醒窗上的候选回复靠这个 bot 发出去（notice::notice_reply 拿不到 bot）。
    crate::notice::set_bot(bot.clone());

    plugin::on_msg(move |event| {
        let bot = bot.clone();

        async move {
            if !need_to_solve(bot.clone(), event.clone()).await {
                return;
            }

            let config = Config::load_or_create(CONFIG_PATH);

            // 非纯文字消息：里面有文件段的交给 files.rs（下载桌面 / 按后缀策略忽略 /
            // 需要时自动打开），其余（图片、表情、语音、混合消息里的文字）一律当没看见。
            // 判据是 segment.kind（kovi-0.13.5/src/message.rs:8-12）。
            if !event.message.iter().all(|segment| segment.kind == "text") {
                if event.message.iter().any(|segment| segment.kind == "file") {
                    crate::files::handle(bot.clone(), event.user_id, &event.message, &config).await;
                }
                return;
            }
            let Some(text) = event.borrow_text() else {
                return;
            };

            // 提醒窗顶栏要显示「备注 [用户名] （QQ号）」。好友列表有 5 分钟缓存，正常情况下
            // 这里只是查一次表；查不到也会退化成「（QQ号）」，不会挡住建窗。
            let sender = get_friend_info(bot.clone(), event.get_sender_id().to_id()).await;

            // 命中关键词就拿它的候选回复；没命中就是空表，提醒窗不显示候选按钮。
            // find_matched_keywords 返回的是 Value 的并集（即候选回复本身），不是关键词名
            // （utils/mod.rs:19-33），正好是这里要推给前端的。
            // 先记一条聊天记录再提醒：托盘「聊天记录」窗口要靠它翻历史，
            // 重放时也用它去重（同一条已经在提醒窗里就不再弹）。
            let history_id = crate::history::record(&event.user_id.to_string(), &sender, text);

            let target = ReplyTarget {
                sender_id: event.user_id,
                message_id: event.message_id,
            };
            deliver(
                text,
                &sender,
                &config,
                target,
                Some(history_id),
                "收到消息",
                false,
            );
        }
    });
}

/// 把一条要提醒的文本按当前场景矩阵送出去；真正又展示了就返回 true。
///
/// 真收到的消息和聊天记录的重放共用这一条路径，区别在 kind（日志前缀）、
/// history_id（去重）和 force_strong（见下）。以前命中关键词就直接
/// reply_and_quote("👍")，那是联调阶段的占位逻辑；强提醒走的是
/// 「用户点候选回复（notice_reply）才真的发出去」。
///
/// force_strong：重放走的那条。重放没有原始消息（ReplyTarget.message_id == 0），
/// 所以一不按场景矩阵分级——不管上课/下课、窗口什么状态，一律强提醒
/// （notice::show 置顶抢焦点），绝不允许落到 show_side；二不给候选回复，
/// 点了也回不到正确的会话。真收到消息（on_msg）传 false，分级路由完全不变。
fn deliver(
    text: &str,
    sender: &str,
    config: &Config,
    target: ReplyTarget,
    history_id: Option<u64>,
    kind: &str,
    force_strong: bool,
) -> bool {
    // 命中关键词就拿它的候选回复；没命中就是空表，提醒窗不显示候选按钮。
    // find_matched_keywords 返回的是 Value 的并集（即候选回复本身），不是关键词名
    // （utils/mod.rs:19-33），正好是这里要推给前端的。
    // 重放恒为空表：message_id=0，没有原消息，点了回复也发不到正确的会话。
    let replies: Vec<String> = if force_strong {
        Vec::new()
    } else {
        find_matched_keywords(text, &config.quick_replies)
            .into_iter()
            .cloned()
            .collect()
    };

    // 场景矩阵决定这条消息怎么提醒：上课状态来自 AstraSchedule/desktop 的 /status
    // （连不上就用配置里的「默认状态」），窗口状态来自前台窗口（win.rs）。
    let (period, state, action) = crate::schedule::current_scene(config);
    // 重放强制走强提醒：不排队、不降级成侧通知。
    let action = if force_strong {
        SceneAction::Strong
    } else {
        action
    };
    // 决策依据必须留下：真机排查「怎么走成侧通知了」全看这一行。
    // 必须走 log 而不是 eprintln!：GUI 版没有控制台，stderr 没人接，只有 log 才落进 app.log
    // （安装版更明显——用户报「上课怎么还弹」时，这一行是唯一能证明 app 当时认为是什么状态的证据）。
    log::info!(
        "{kind}：{} / {} → {}{}",
        period.label(),
        state.label(),
        action.label(),
        if force_strong { "（重放：强制强提醒）" } else { "" }
    );

    match action {
        SceneAction::Strong => crate::notice::show(text, sender, replies, target, history_id),
        // 侧通知只显示文字、不给候选回复，所以不传 target（target 仍要给延后队列）。
        SceneAction::Side => crate::notice::show_side(text, sender, history_id),
        // 「延后至下课」先存进队列，等离开了上课那一格再放行（defer.rs）。
        // 进队列就算处理掉了：稍后会自己弹出来。
        SceneAction::Postpone => {
            crate::defer::push(text.to_string(), sender.to_string(), replies, target, history_id);
            true
        }
        // 忽略：这条消息只进聊天记录（上面的 history::record 已经记过了），不弹任何提醒。
        // 返回 false = 「这次没有展示」，调用方（聊天记录重放）据此不重复计数。
        SceneAction::Ignore => false,
    }
}

/// 重放一条聊天记录：效果和「这条消息刚刚又收到了」完全一样。
///
/// 没有原始消息，所以 message_id 给 0——回复时只发文本、不带引用段。
/// 已经在提醒窗里显示着的那条会被去重挡掉，这里就返回 false。
///
/// 和真收到消息的唯一区别：force_strong = true。重放是用户主动在聊天记录里点的，
/// 必须当场看见——所以无视场景矩阵走强提醒，且不给候选回复（见 deliver）。
pub fn replay(entry: &crate::history::Entry) -> bool {
    let Ok(sender_id) = entry.qq.parse::<i64>() else {
        eprintln!("聊天记录里的 QQ 号不合法，跳过：{}", entry.qq);
        return false;
    };
    let config = Config::load_or_create(CONFIG_PATH);
    deliver(
        &entry.text,
        &entry.name,
        &config,
        ReplyTarget {
            sender_id,
            message_id: 0,
        },
        Some(entry.id),
        "重放消息",
        true,
    )
}

/// 启动 kovi：等价于旧仓库 `src/main.rs` 的那几行，外加挂插件。
///
/// 旧仓库写的是 `build_bot!(driver; kovi_plugin_cmd, hello)`，宏收的是 crate 名的 ident
/// （`$plugin::__kovi_build_plugin()`），而本仓库的插件就在自己 crate 根部，ident 表达不了
/// 「本 crate」，所以这里用宏内部同样的几个公开 API 手挂。
/// 连不上就一直重试，不设上限：第一次跑起来的 OneBot 框架要人工同意条款、扫码登录 QQ，
/// 这期间连不上是常态。连不上时 kovi 的 `run()` 会很快返回
/// （kovi-0.13.5/src/bot/run/connect.rs 的 event_connect 拿不到通道就直接
/// send(Exit)），所以这里可以踏实循环。
pub async fn start() {
    kovi::logger::try_set_logger_use_env();

    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        // 每轮重读配置：用户可能在界面上改了启动文件路径，框架也可能刚生成自己的配置。
        // 顺手确保托管的那份框架起着（端口已经在听就直接用现成的）。
        let config = Config::load_or_create(CONFIG_PATH);
        crate::onebot::ensure_running(&config);

        // 每一轮都丢进独立的 task 跑：kovi 内部还有几十处 expect（例如
        // kovi-onebot/src/driver/config.rs:141 的无终端交互提问），一个 panic 只会
        // 结束这一次尝试，不会把外层这个重连循环一起带走。
        let attempt_config = config.clone();
        let attempt_task = kovi::tokio::spawn(async move {
            // 连接参数：用户在「连接配置」里手填的优先，留空的项从框架自己的配置里补
            // （地址 + access token），都没有才回落到项目里那份 kovi.conf.toml。
            let driver =
                kovi_onebot::OneBotDriver::new(crate::onebot::driver_config(&attempt_config));
            // kovi 自己的配置（只有 main_admin / admins / debug 三个字段，本项目的插件用不到管理员）
            // 直接在代码里给：Bot::build 收的是 `C: AsRef<KoviConf>`（kovi-0.13.5/bot.rs:58），
            // 没必要落一个 kovi.conf.toml——那个文件的读取路径（config/kovi_conf.rs:123）在缺文件时
            // 会走 dialoguer 交互提问（:44），GUI 进程的 stdin 是死的，一挂就是整个启动流程。
            let kovi_conf = KoviConf::new(ID::new(String::new()), None, false);
            let mut bot = kovi::Bot::build(kovi_conf, driver);
            bot.mount_plugin(crate::get_plugin()); // 本 crate 的插件（宏在根部生成）
            bot.mount_plugin(kovi_plugin_cmd::get_plugin());
            bot.set_plugin_startup_use_file_ref(); // 按插件名读 kovi.plugin.toml

            bot.run().await // 阻塞到连接断开
        });

        if let Err(error) = attempt_task.await {
            eprintln!("这一轮 bot 任务异常退出：{error}");
        }

        // 句柄作废：顶栏那盏灯要跟着转红，点候选回复也不该再往死连接上写。
        crate::notice::clear_bot();
        eprintln!(
            "第 {attempt} 次连接已断开，5s 后重试（首次安装要在托盘「连接配置」里同意条款并登录 QQ）"
        );
        kovi::tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
