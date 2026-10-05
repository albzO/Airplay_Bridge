use homepod_test::{backend, capture, convert, discovery, failure, live};

use std::{env, path::PathBuf};

fn project_root() -> PathBuf {
    PathBuf::from(env::var_os("APPDATA").expect("APPDATA 未设置"))
        .join("AirPlay Hub")
        .join("cli")
}

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "错误：{}",
            failure::describe(&error.to_string(), "INTERNAL_ERROR")
        );
        let exit = error
            .downcast_ref::<failure::SessionError>()
            .map_or_else(|| failure::exit_for(&error.to_string()), |e| e.exit);
        std::process::exit(exit);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    homepod_test::data_dir::prepare()?;
    std::fs::create_dir_all(project_root())?;
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("stream") | Some("stream-stereo") => {
            let stereo = args[0] == "stream-stereo";
            let name = if stereo {
                value(&args, "--left").ok_or("请指定 --left <设备名称>")?
            } else {
                value(&args, "--device").ok_or("请指定 --device <设备名称>")?
            };
            let seconds =
                parse_value::<u64>(value(&args, "--seconds").unwrap_or("600"), "--seconds")?;
            let latency = parse_value::<u32>(
                value(&args, "--latency-ms").unwrap_or("2000"),
                "--latency-ms",
            )?;
            let buffer =
                parse_value::<u32>(value(&args, "--buffer-ms").unwrap_or("128"), "--buffer-ms")?;
            if seconds > 86400 || !(250..=2000).contains(&latency) || !(64..=512).contains(&buffer)
            {
                return Err(
                    "seconds 应为 0～86400（0 持续运行），latency-ms 250～2000，buffer-ms 64～512"
                        .into(),
                );
            }
            if stereo {
                live::run_stereo(
                    &project_root(),
                    name,
                    value(&args, "--right").ok_or("请指定 --right <设备名称>")?,
                    seconds,
                    latency,
                    buffer,
                    value(&args, "--endpoint"),
                )
            } else {
                live::run(
                    &project_root(),
                    name,
                    seconds,
                    latency,
                    buffer,
                    value(&args, "--endpoint"),
                )
            }
        }
        Some("convert-pcm") => {
            let input = value(&args, "--input").ok_or("请指定 --input <采集 float32 WAV>")?;
            convert::wave(std::path::Path::new(input)).map(|_| ())
        }
        Some("audio-devices") => capture::list(&project_root()),
        Some("capture") => {
            let seconds =
                parse_value::<u64>(value(&args, "--seconds").unwrap_or("10"), "--seconds")?;
            if !(1..=60).contains(&seconds) {
                return Err("采集时长应在 1～60 秒之间".into());
            }
            let wave = capture::record(&project_root(), seconds, value(&args, "--endpoint"))?;
            if args.iter().any(|arg| arg == "--convert") {
                convert::wave(&wave)?;
            }
            Ok(())
        }
        Some("replay") => {
            let name = value(&args, "--device").ok_or("请指定 --device <设备名称>")?;
            let seconds =
                parse_value::<u64>(value(&args, "--seconds").unwrap_or("10"), "--seconds")?;
            if !(1..=60).contains(&seconds) {
                return Err("录制时长应在 1～60 秒之间".into());
            }
            println!("先录制所选音频来源，再转换并回放到 HomePod；这是有限录音回放测试。");
            let wave = capture::record(&project_root(), seconds, value(&args, "--endpoint"))?;
            let pcm = convert::wave(&wave)?;
            backend::test(&project_root(), name, "ptp", true, false, Some(&pcm))
        }
        Some("play-pcm") => {
            let name = value(&args, "--device").ok_or("请指定 --device <HomePod 名称>")?;
            let input = value(&args, "--input").ok_or("请指定 --input <本工具生成的 .pcm 文件>")?;
            backend::test(
                &project_root(),
                name,
                "ptp",
                true,
                false,
                Some(std::path::Path::new(input)),
            )
        }
        Some("discover") => {
            let seconds =
                parse_value::<u64>(value(&args, "--seconds").unwrap_or("10"), "--seconds")?;
            if !(1..=60).contains(&seconds) {
                return Err("发现时长应在 1～60 秒之间".into());
            }
            discovery::discover(&project_root(), seconds)
        }
        Some("test") | Some("tone") => {
            let name = value(&args, "--device").ok_or("请指定 --device <设备名称>")?;
            let timing = value(&args, "--timing").unwrap_or("ptp");
            if timing != "ptp" && timing != "ntp" {
                return Err("timing 应为 ptp 或 ntp".into());
            }
            backend::test(
                &project_root(),
                name,
                timing,
                !args.iter().any(|a| a == "--no-password"),
                args[0] == "tone",
                None,
            )
        }
        Some(command) if command != "--help" && command != "help" => {
            Err(format!("[INVALID_ARGUMENT] 未知命令：{command}；请使用 --help").into())
        }
        _ => {
            println!(
                "HomePod 测试工具\n\
                      homepod-test discover [--seconds 10]\n\
                      homepod-test test --device <设备名称> [--timing ptp|ntp] [--no-password]\n\
                      homepod-test tone --device <设备名称> [--timing ptp|ntp]\n\
                      homepod-test audio-devices\n\
                      homepod-test capture [--endpoint <Windows 端点 ID>] [--seconds 10] [--convert]\n\
                      homepod-test convert-pcm --input <采集 float32.wav>\n\
                      homepod-test play-pcm --device <设备名称> --input <converted.pcm>\n\
                      homepod-test replay --device <设备名称> [--seconds 10] (先录制，再回放)\n\
                      homepod-test stream --device <设备名称> [--endpoint <Windows 端点 ID>] [--seconds 600|0] [--latency-ms 2000] [--buffer-ms 128] (Ctrl+C 停止)\n\
                       homepod-test stream-stereo --left <设备名称> --right <另一个设备名称> [--seconds 600|0] [--latency-ms 2000] [--buffer-ms 128]\n\
                      先 discover，再 test；密码由后端在本机隐藏输入，不写入配置或命令行。"
            );
            Ok(())
        }
    }
}

fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

fn parse_value<T: std::str::FromStr>(
    text: &str,
    flag: &str,
) -> Result<T, Box<dyn std::error::Error>> {
    text.parse()
        .map_err(|_| format!("[INVALID_ARGUMENT] {flag} 必须是有效整数").into())
}
