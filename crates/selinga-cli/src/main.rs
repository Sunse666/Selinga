use clap::{Parser, Subcommand};
use selinga_capture::Region;
#[cfg(windows)]
use selinga_capture::ScreenCapture;
#[cfg(windows)]
use selinga_ocr::Ocr;
#[cfg(windows)]
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
        /// OCR engine: "win" (Windows.Media.Ocr, default) or "vlm"
        /// (OpenAI-compatible vision model, reads [vlm] from settings.toml).
        #[arg(long = "engine", default_value = "win")]
        engine: String,
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
            engine,
        } => {
            if let Err(err) = run_capture_ocr(
                &region,
                lang.as_deref(),
                boxes,
                dump.as_deref(),
                translate,
                tl.as_deref(),
                &engine,
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
    engine: &str,
) -> Result<(), String> {
    if engine != "win" && engine != "vlm" {
        return Err(format!(
            "unknown engine {engine:?} (expected \"win\" or \"vlm\")"
        ));
    }
    let region = parse_region(region)?;

    #[cfg(windows)]
    {
        let capture = selinga_capture::win32::GdiCapture::new().map_err(|e| e.to_string())?;
        let frame = capture.capture_region(region).map_err(|e| e.to_string())?;

        if let Some(path) = dump {
            write_bmp(path, &frame).map_err(|e| format!("dump failed: {e}"))?;
            eprintln!("frame dumped to {path}");
        }

        let lines = if engine == "vlm" {
            let endpoint = load_vlm_endpoint()?;
            eprintln!("engine: vlm ({} @ {})", endpoint.model, endpoint.base_url);
            selinga_ocr::vlm::VlmOcr::new(endpoint)
                .recognize(&frame)
                .map_err(|e| e.to_string())?
        } else {
            let engine = selinga_ocr::win32::WinOcr::new(lang).map_err(|e| e.to_string())?;
            engine.recognize(&frame).map_err(|e| e.to_string())?
        };

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
        let _ = (lang, boxes, dump, translate, tl, engine);
        Err(format!(
            "capture-ocr is Windows-only in M0 (region {region:?})"
        ))
    }
}

/// Read [vlm] from the same settings.toml the app uses and build an
/// endpoint. Requires enabled = true plus non-empty base_url and model.
fn load_vlm_endpoint() -> Result<selinga_ocr::vlm::VlmEndpoint, String> {
    #[derive(serde::Deserialize)]
    struct VlmSection {
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        base_url: String,
        #[serde(default)]
        api_key: String,
        #[serde(default)]
        model: String,
    }
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        vlm: Option<VlmSection>,
    }

    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"));
    let path = base
        .map(|b| b.join("selinga").join("settings.toml"))
        .ok_or_else(|| "cannot locate settings directory".to_owned())?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let file: File =
        toml::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let vlm = file.vlm.ok_or("no [vlm] section in settings.toml")?;
    if !vlm.enabled {
        return Err("vlm.enabled = false in settings.toml".to_owned());
    }
    if vlm.base_url.trim().is_empty() || vlm.model.trim().is_empty() {
        return Err("vlm.base_url and vlm.model must be set in settings.toml".to_owned());
    }
    Ok(selinga_ocr::vlm::VlmEndpoint {
        base_url: vlm.base_url.trim().to_owned(),
        api_key: Some(vlm.api_key.trim().to_owned()).filter(|k| !k.is_empty()),
        model: vlm.model.trim().to_owned(),
    })
}

#[cfg(windows)]
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
