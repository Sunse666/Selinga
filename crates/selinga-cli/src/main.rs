use clap::{Parser, Subcommand};
use selinga_capture::{Region, ScreenCapture};
use selinga_ocr::Ocr;
use selinga_translate::Translator;

#[derive(Parser)]
#[command(name = "selinga-cli", version, about = "Selinga M0 debug CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    CaptureOcr {
        #[arg(long = "region")]
        region: String,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        boxes: bool,
        #[arg(long)]
        dump: Option<String>,
        #[arg(long)]
        translate: bool,
        #[arg(long)]
        tl: Option<String>,
    },
}

fn main() {
    selinga_platform::enable_utf8_console();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::CaptureOcr {
            region,
            lang,
            boxes,
            dump,
            translate,
            tl,
        } => {
            if let Err(err) = run_capture_ocr(
                &region,
                lang.as_deref(),
                boxes,
                dump.as_deref(),
                translate,
                tl.as_deref(),
            ) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_capture_ocr(
    region: &str,
    lang: Option<&str>,
    boxes: bool,
    dump: Option<&str>,
    translate: bool,
    tl: Option<&str>,
) -> Result<(), String> {
    let region = parse_region(region)?;

    #[cfg(windows)]
    {
        let capture = selinga_capture::win32::GdiCapture::new().map_err(|e| e.to_string())?;
        let frame = capture.capture_region(region).map_err(|e| e.to_string())?;

        if let Some(path) = dump {
            write_bmp(path, &frame).map_err(|e| format!("dump failed: {e}"))?;
            eprintln!("frame dumped to {path}");
        }

        let engine = selinga_ocr::win32::WinOcr::new(lang).map_err(|e| e.to_string())?;
        let lines = engine.recognize(&frame).map_err(|e| e.to_string())?;

        if lines.is_empty() {
            println!("(no text recognized)");
            return Ok(());
        }
        let sources: Vec<String> = lines.iter().map(|l| l.text.clone()).collect();
        let translated = if translate {
            let target = tl.unwrap_or("zh-CN");
            use selinga_translate::FallbackTranslator;
            let chain = FallbackTranslator::new(vec![
                Box::new(selinga_translate::tencent::TransmartTranslator::new(target)),
                Box::new(selinga_translate::google::GoogleTranslator::new(target)),
                Box::new(selinga_translate::mymemory::MyMemoryTranslator::new(
                    "en-US", target,
                )),
            ]);
            match chain.translate_lines(&sources) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("warning: {e}");
                    sources.clone()
                }
            }
        } else {
            sources.clone()
        };
        for (line, tr) in lines.into_iter().zip(translated) {
            if translate {
                println!("{} -> {}", line.text, tr);
            } else {
                println!("{}", line.text);
            }
            if boxes {
                for word in line.words {
                    println!(
                        "    [{:6.1},{:6.1} {:6.1}x{:6.1}] {}",
                        word.rect.x, word.rect.y, word.rect.w, word.rect.h, word.text
                    );
                }
            }
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = (lang, boxes, dump, translate, tl);
        Err(format!(
            "capture-ocr is Windows-only in M0 (region {region:?})"
        ))
    }
}

fn write_bmp(path: &str, frame: &selinga_capture::Frame) -> std::io::Result<()> {
    use std::io::Write;

    let (w, h) = (frame.width, frame.height);
    let data_len = frame.bgra.len() as u32;
    let file_len = 54 + data_len;

    let mut out = Vec::with_capacity(file_len as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_len.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);

    let stride = (w as usize) * 4;
    for row in frame.bgra.chunks_exact(stride).rev() {
        out.write_all(row)?;
    }

    std::fs::write(path, out)
}

fn parse_region(spec: &str) -> Result<Region, String> {
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(format!("region must be x,y,w,h, got {spec:?}"));
    }
    let parse = |s: &str| s.parse::<i64>().map_err(|_| format!("bad number {s:?}"));
    let x = parse(parts[0])?;
    let y = parse(parts[1])?;
    let w = parse(parts[2])?;
    let h = parse(parts[3])?;
    if w <= 0 || h <= 0 {
        return Err("width and height must be positive".to_owned());
    }
    Ok(Region {
        x: x as i32,
        y: y as i32,
        w: w as u32,
        h: h as u32,
    })
}
