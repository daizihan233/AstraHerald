//! 安装时的命令行预设：setup.exe /S /AUTOSTART=1 /WHITELIST=123,456 ...
//!
//! 安装器把命令行参数摊平成一份 key=value 的文件（$INSTDIR\install-preset.ini，补丁见
//! scripts/sync-nsis-template.mjs），首次启动时由这里并进 config.json，然后删掉这份文件。
//!
//! 为什么是 ini 而不是直接写 JSON：NSIS 里写 JSON 字符串得转义反斜杠，
//! C:\SnowLuma\launcher.bat 这种路径会变成非法转义。

use std::fs;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::config::{Config, CONFIG_PATH};

/// 安装器留下的预设文件。它在安装目录里，而安装目录就是我们的工作目录（config.rs 的锚定）。
pub const PRESET_PATH: &str = "install-preset.ini";

/// 把安装时的预设并进配置。返回是否真的写入了配置。
///
/// 只在文件存在时做事：普通安装、升级、手动跑都不会有这个文件。
pub fn apply_if_any() -> bool {
    let path = Path::new(PRESET_PATH);
    if !path.is_file() {
        return false;
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("读安装预设 {PRESET_PATH} 失败：{error}");
            return false;
        }
    };

    let overlay = parse(&text);
    if overlay.is_empty() {
        eprintln!("安装预设里没有可用的项，已忽略");
        let _ = fs::remove_file(path);
        return false;
    }

    let mut base = match serde_json::to_value(Config::load_or_create(CONFIG_PATH)) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("读现有配置失败：{error}，安装预设先留着");
            return false;
        }
    };
    merge(&mut base, Value::Object(overlay));
    let config: Config = match serde_json::from_value(base) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("安装预设跟配置对不上：{error}，已忽略");
            let _ = fs::remove_file(path);
            return false;
        }
    };
    if let Err(error) = config.validate() {
        eprintln!("安装预设里的号码有问题：{error}，已忽略");
        let _ = fs::remove_file(path);
        return false;
    }

    // 先写配置再删预设：反过来的话，写配置失败就再也读不到预设了。
    if let Err(error) = config.save(CONFIG_PATH) {
        eprintln!("写入配置失败：{error}，安装预设先留着，下次启动再试");
        return false;
    }
    if let Err(error) = fs::remove_file(path) {
        eprintln!("删除 {PRESET_PATH} 失败：{error}");
    }
    eprintln!("已应用安装时的预设并写入 {CONFIG_PATH}");
    true
}

/// 把 ini 摊成一份 JSON overlay（只放用户真的传了的项）。
fn parse(text: &str) -> Map<String, Value> {
    let mut root = Map::new();
    let mut onebot = Map::new();
    let mut side = Map::new();
    let mut updater = Map::new();

    for line in text.lines() {
        let line = line.trim();
        // 空行、注释、[app] 这种段名都不关心：一共就这几个键，段名没有意义。
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[')
        {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            eprintln!("安装预设里有看不懂的行：{line}");
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        // 空值 = 没传（安装器也只写非空的行），所以这里不处理「清空某项」。
        if value.is_empty() {
            continue;
        }

        match key.as_str() {
            "autostart" => match truthy(value) {
                Some(on) => root.insert("autostart".to_string(), json!(on)),
                None => {
                    eprintln!("安装预设：autostart={value} 不是开关值，忽略");
                    continue;
                }
            },
            "period" | "default_period" => {
                let period = match value {
                    "in_class" | "class" => "in_class",
                    "after_class" | "after" => "after_class",
                    other => {
                        eprintln!("安装预设：period={other} 不是 in_class / after_class，忽略");
                        continue;
                    }
                };
                root.insert("default_period".to_string(), json!(period))
            }
            "whitelist" => root.insert("whitelist".to_string(), json!(ids(value))),
            "blacklist" => root.insert("blacklist".to_string(), json!(ids(value))),
            "onebot_startup" => onebot.insert("startup".to_string(), json!(value)),
            "onebot_host" => onebot.insert("host".to_string(), json!(value)),
            "onebot_port" => onebot.insert("port".to_string(), json!(value)),
            "onebot_path" => onebot.insert("path".to_string(), json!(value)),
            "onebot_token" => onebot.insert("access_token".to_string(), json!(value)),
            "side_duration" | "side_duration_secs" => match value.parse::<u64>() {
                Ok(secs) => side.insert("duration_secs".to_string(), json!(secs)),
                Err(_) => {
                    eprintln!("安装预设：side_duration={value} 不是秒数，忽略");
                    continue;
                }
            },
            "updater" => match truthy(value) {
                Some(on) => updater.insert("enabled".to_string(), json!(on)),
                None => {
                    eprintln!("安装预设：updater={value} 不是开关值，忽略");
                    continue;
                }
            },
            "update_source" => updater.insert("source".to_string(), json!(value)),
            other => {
                eprintln!("安装预设里有不认识的项：{other}（忽略）");
                continue;
            }
        };
    }

    if !onebot.is_empty() {
        root.insert("onebot".to_string(), Value::Object(onebot));
    }
    if !side.is_empty() {
        root.insert("side".to_string(), Value::Object(side));
    }
    if !updater.is_empty() {
        root.insert("updater".to_string(), Value::Object(updater));
    }
    root
}

/// 一层一层并：只覆盖预设里提到的字段，onebot 里没提的项保持原样。
fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                merge(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, patch) => *base = patch,
    }
}

/// 开关值：1/true/yes/on 算开，0/false/no/off 算关，其余返回 None（调用方打日志忽略）。
fn truthy(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "y" => Some(true),
        "0" | "false" | "no" | "off" | "n" => Some(false),
        _ => None,
    }
}

/// 逗号/分号/空白分隔的号码串 → 字符串数组（配置里的 ID 必须是字符串，见 config.rs 的 validate）。
fn ids(value: &str) -> Vec<String> {
    value
        .split([',', ';', ' ', '\t'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[app]
autostart=1
period=after_class
whitelist=123, 456
onebot_startup=D:\\SnowLuma\\launcher.bat
onebot_port=3001
side_duration=20
updater=0
update_source=https://example.com/latest.json
";

    #[test]
    fn parses_the_installer_preset() {
        let overlay = parse(SAMPLE);
        assert_eq!(overlay["autostart"], json!(true));
        assert_eq!(overlay["default_period"], json!("after_class"));
        assert_eq!(overlay["whitelist"], json!(["123", "456"]));
        assert_eq!(
            overlay["onebot"]["startup"],
            json!("D:\\SnowLuma\\launcher.bat")
        );
        assert_eq!(overlay["onebot"]["port"], json!("3001"));
        assert_eq!(overlay["side"]["duration_secs"], json!(20));
        assert_eq!(overlay["updater"]["enabled"], json!(false));
        assert_eq!(
            overlay["updater"]["source"],
            json!("https://example.com/latest.json")
        );
    }

    #[test]
    fn skips_lines_it_does_not_understand() {
        let overlay = parse(
            "# 注释
[app]
; 另一种注释
unknown=1
side_duration=abc
autostart=maybe
这样一行没有等号
",
        );
        assert!(overlay.is_empty(), "只该剩下认识且合法的项：{overlay:?}");
    }

    #[test]
    fn merge_keeps_the_fields_the_preset_did_not_mention() {
        let mut base = json!({
            "autostart": true,
            "onebot": { "startup": "", "host": "127.0.0.1", "port": "" },
            "side": { "duration_secs": 15 },
        });
        merge(&mut base, Value::Object(parse("onebot_startup=foo.exe
side_duration=30")));
        assert_eq!(base["onebot"]["startup"], json!("foo.exe"));
        assert_eq!(base["onebot"]["host"], json!("127.0.0.1"), "没提到的字段不能被抹掉");
        assert_eq!(base["onebot"]["port"], json!(""));
        assert_eq!(base["side"]["duration_secs"], json!(30));
        assert_eq!(base["autostart"], json!(true));
    }
}
