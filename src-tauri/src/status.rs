//! 配置页顶栏那两盏连接灯（OneBot / 课表）。
//!
//! 不新增状态源，只把现成的东西拼起来看一眼：
//! - OneBot：能算出要连的端点（框架配置里读的或用户在连接配置里手填的）、那个端口在听、
//!   bot 插件入口跑过（说明真连上了）；
//! - 课表：schedule 的 2s 轮询最近一轮有没有打通 /status。
//!
//! 灯只说明「现在能不能用」，细节（端口、配置来自哪个目录、为什么连不上）放在悬停提示里。

use serde::Serialize;

use crate::config::{Config, CONFIG_PATH};
use crate::{notice, onebot, schedule};

/// 一盏灯：能不能用 + 一句人话（悬停时显示）。
#[derive(Serialize)]
pub struct Link {
    pub ok: bool,
    pub detail: String,
}

#[derive(Serialize)]
pub struct Status {
    pub onebot: Link,
    pub schedule: Link,
    /// 现在按上课还是下课算，以及这个结论来自课表还是配置里的默认值。
    pub period: String,
}

#[tauri::command]
pub fn connection_status() -> Status {
    // 配置每 3s 被读一次：就一个小文件，和 bot 每条消息读一次是同一套用法。
    let config = Config::load_or_create(CONFIG_PATH);
    let live = schedule::is_live();
    Status {
        onebot: onebot_link(&config),
        schedule: schedule_link(live),
        period: format!(
            "{}{}",
            schedule::current_period(&config).label(),
            if live { "（课表）" } else { "（默认）" }
        ),
    }
}

fn onebot_link(config: &Config) -> Link {
    let Some((from, endpoint)) = onebot::effective_endpoint(&config.onebot) else {
        return Link {
            ok: false,
            detail: "还没指定 OneBot 框架的启动文件（配置页 → 连接配置）".to_string(),
        };
    };
    let addr = format!("{}:{}", endpoint.host, endpoint.port);
    let source = match &from {
        Some(dir) => format!("（配置来自 {}）", dir.display()),
        None => "（手填）".to_string(),
    };
    if !onebot::port_open(&endpoint.host, endpoint.port) {
        return Link {
            ok: false,
            detail: format!("{addr} 没在监听{source}"),
        };
    }
    // ponytail: bot 句柄拿到后不会清，断线后这盏灯会一直亮绿；升级路径是订阅 kovi 的连接事件。
    if notice::bot_handle().is_none() {
        return Link {
            ok: false,
            detail: format!("{addr} 在监听，但消息通道还没连上{source}"),
        };
    }
    Link {
        ok: true,
        detail: format!("{addr}{source}"),
    }
}

fn schedule_link(live: bool) -> Link {
    if live {
        Link {
            ok: true,
            detail: "AstraSchedule 状态接口 127.0.0.1:3011 正常".to_string(),
        }
    } else {
        Link {
            ok: false,
            detail: "连不上 127.0.0.1:3011，场景按配置里的「默认状态」走".to_string(),
        }
    }
}
