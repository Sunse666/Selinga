# Selinga

**Sel**ect + **Lingo** — 跨平台屏幕翻译工具：一个可拖动的透明方框，框内内容自动 OCR 并翻译。

## 技术栈

- Rust (stable) + egui/winit
- OCR：系统级离线 OCR（Windows.Media.Ocr；macOS Apple Vision / Linux PaddleOCR ONNX 预留）
- 翻译：`Translator` trait 多后端（Google 免费端点起步）

## 开发

参见 [DEVELOPMENT.md](DEVELOPMENT.md)（注释/Git/CI 规范与里程碑）。

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## M0 调试工具

```bash
# 截取屏幕区域（物理像素）并输出 OCR 文本
selinga-cli capture-ocr --region 100,200,800,400
selinga-cli capture-ocr --region 100,200,800,400 --lang zh-CN --boxes
```
