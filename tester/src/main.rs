use homepod_test::{backend, capture, convert, discovery, failure, live};

use std::{env, path::PathBuf};

fn project_root() -> PathBuf {
    // Packaged executable sits beside probe.exe and devices.json in dist/.
    env::current_exe().unwrap().parent().unwrap().to_path_buf()
}

fn main() {
    if let Err(error) = run() {
        eprintln!("错误：{error}");
        let exit = error
            .downcast_ref::<failure::SessionError>()
            .map_or(1, |e| e.exit);
        std::process::exit(exit);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("stream-b3") | Some("stream-stereo") => {
            let stereo = args[0] == "stream-stereo";
            let name = if stereo {
                value(&args, "--left").unwrap_or("黑球")
            } else {
                value(&args, "--device").ok_or("请指定 --device 黑球 或 --device 黄球")?
            };
            let seconds = value(&args, "--seconds").unwrap_or("600").parse::<u64>()?;
            let latency = value(&args, "--latency-ms")
                .unwrap_or("2000")
                .parse::<u32>()?;
            let buffer = value(&args, "--buffer-ms")
                .unwrap_or("128")
                .parse::<u32>()?;
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
                    value(&args, "--right").unwrap_or("黄球"),
                    seconds,
                    latency,
                    buffer,
                )
            } else {
                live::run(&project_root(), name, seconds, latency, buffer)
            }
        }
        Some("convert-pcm") => {
            let input = value(&args, "--input").ok_or("请指定 --input <B3 float32 WAV>")?;
            convert::wave(std::path::Path::new(input)).map(|_| ())
        }
        Some("audio-devices") => capture::list(&project_root()),
        Some("capture-b3") => {
            let seconds = value(&args, "--seconds").unwrap_or("10").parse::<u64>()?;
            if !(1..=60).contains(&seconds) {
                return Err("采集时长应在 1～60 秒之间".into());
            }
            let wave = capture::b3(&project_root(), seconds)?;
            if args.iter().any(|arg| arg == "--convert") {
                convert::wave(&wave)?;
            }
            Ok(())
        }
        Some("replay-b3") => {
            let name = value(&args, "--device").ok_or("请指定 --device 黑球 或 --device 黄球")?;
            let seconds = value(&args, "--seconds").unwrap_or("10").parse::<u64>()?;
            if !(1..=60).contains(&seconds) {
                return Err("录制时长应在 1～60 秒之间".into());
            }
            println!("先录制 B3，再转换并回放到 HomePod；这是有限录音回放测试。");
            let wave = capture::b3(&project_root(), seconds)?;
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
            let seconds = value(&args, "--seconds").unwrap_or("10").parse::<u64>()?;
            if !(1..=60).contains(&seconds) {
                return Err("发现时长应在 1～60 秒之间".into());
            }
            discovery::discover(&project_root(), seconds)
        }
        Some("test") | Some("tone") => {
            let name = value(&args, "--device").ok_or("请指定 --device 黑球 或 --device 黄球")?;
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
        _ => {
            println!(
                "HomePod 测试工具\n\
                      homepod-test discover [--seconds 10]\n\
                      homepod-test test --device 黑球 [--timing ptp|ntp] [--no-password]\n\
                      homepod-test tone --device 黑球 [--timing ptp|ntp]\n\
                      homepod-test audio-devices\n\
                      homepod-test capture-b3 [--seconds 10] [--convert]\n\
                      homepod-test convert-pcm --input <B3 float32.wav>\n\
                      homepod-test play-pcm --device 黑球 --input <converted.pcm>\n\
                      homepod-test replay-b3 --device 黑球 [--seconds 10] (先录制，再回放)\n\
                      homepod-test stream-b3 --device 黑球 [--seconds 600|0] [--latency-ms 2000] [--buffer-ms 128] (Ctrl+C 停止)\n\
                       homepod-test stream-stereo [--left 黑球] [--right 黄球] [--seconds 600|0] [--latency-ms 2000] [--buffer-ms 128]\n\
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
