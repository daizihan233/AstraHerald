//! 只跑 bot、不起 GUI 的调试入口：`cargo run --bin bot-only`。
//! 日常 B 端联调用它验证 kovi 能否连上 OneBot 服务端，不受窗口影响。

fn main() {
    let rt = kovi::tokio::runtime::Runtime::new().expect("create tokio runtime");
    rt.block_on(async {
        astra_herald_lib::anchor_to_data_dir();
        let _logger_guard = owl_logger::init();
        // 内部是无限重连循环，正常情况下不会返回。
        astra_herald_lib::bot::start().await;
    });
}
