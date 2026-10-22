# Selinga

**Sel**ect + **Lingo** — 启动时框选屏幕任意区域，生成一个可拖动的透明方框，内容自动离线 OCR 并翻译，译文直接覆盖显示。

## 功能

- **启动即框选**：程序打开后屏幕定格（Snipaste 式），拖出矩形即可在该位置生成翻译方框，Esc/右键取消则用默认大小；托盘"重新框选"随时可换区域
- **拖框即译**：方框拖到哪译到哪；内容变化自动重新识别，静止内容走帧哈希 + LRU 双层缓存，零重复开销
- **离线 OCR**：Windows.Media.Ocr 系统级识别（中文/英文/日文等，随系统语言包），不上传画面
- **多后端翻译链**：腾讯 TranSmart（免 key 国内直连）→ Google → MyMemory，自动降级，失败不卡 UI
- **AI 增强（可选）**：视觉大模型同时接管**识别与翻译**（任意 OpenAI 兼容端点：云端 API、硅基流动、本地 Ollama 均可），识别失败自动回退本地 OCR，译文链路照常多级降级
- **双语对照 / 纯译文**两种显示模式，字号自适应，一键复制
- **系统托盘 + 全局热键**：方框不在焦点也能控制
- **设置持久化**：目标语言、显示模式、暂停状态存 TOML，重启保留
- **零依赖分发**：单文件可执行程序，无需安装运行时

## 快捷键与托盘

| 操作 | 热键 | 托盘菜单 |
|---|---|---|
| 隐藏 / 显示方框 | `Ctrl+Alt+H` | 隐藏方框 |
| 暂停 / 恢复识别 | `Ctrl+Alt+P` | 暂停识别 |
| 强制重新识别 | 方框工具栏 刷新图标 | 重新识别 |
| 切换 AI 识别 | — | AI 识别：开/关 |
| 重新框选区域 | — | 重新框选 |
| 退出 | 方框上 `Alt+F4` | 退出 |

方框悬停时右缘出现图标工具栏：切换显示模式、复制结果、目标语言（EN/ZH/JA/KO 文字代码）、重新识别、暂停/恢复；悬停图标有功能提示。

## 安装

从 [Releases](../../releases) 下载对应平台的单文件程序（Windows `selinga.exe`），直接运行即可。

从源码构建：

```bash
git clone https://github.com/Sunse666/Selinga.git
cd Selinga
cargo build --release -p selinga-app
# 产物：target/release/selinga.exe
```

要求 Rust 1.97.1（见 `rust-toolchain.toml`，cargo 会自动拉取）。

## 设置

配置文件位于 `%APPDATA%\selinga\settings.toml`（Windows）或 `~/.config/selinga/settings.toml`（Linux/macOS），程序运行时自动维护；启用 VLM 需手动追加：

```toml
[vlm]
enabled = true
base_url = "http://localhost:11434/v1"   # OpenAI 兼容端点，本地 Ollama 示例
api_key = ""                              # 云端服务才需要；留空 = 无鉴权
model = "qwen2.5:7b"
```

> **隐私提示**：本地 OCR 不上传画面；但译文请求会把**识别出的文本**发送到腾讯 TranSmart /
> Google / MyMemory 的公共接口。启用 `[vlm]` 后，**框选区域的截图会以 PNG 形式上上传**到
> 你配置的端点——若内容敏感，请指向本地 Ollama 等自建服务。

保存后重启生效。`enabled = true` 时视觉大模型同时接管识别与翻译：截图送 VLM 转录（失败自动回退 Windows OCR），译文链路变为 TranSmart → **VLM** → Google → MyMemory。托盘"AI 识别"可在运行时随时开关（状态写回配置），不用重启。

## 命令行调试工具

```bash
cargo run -p selinga-cli -- capture-ocr --region 100,200,800,400
cargo run -p selinga-cli -- capture-ocr --region 100,200,800,400 --lang zh-CN --boxes
cargo run -p selinga-cli -- capture-ocr --region 100,200,800,400 --translate --tl zh-CN
cargo run -p selinga-cli -- capture-ocr --region 100,200,800,400 --engine vlm   # 读取 settings.toml [vlm]
```

## 平台支持

| 平台 | 状态 |
|---|---|
| Windows | ✅ 完整功能（GDI 截屏 + Windows.Media.Ocr） |
| Linux / macOS | ⚠️ 仅保证编译通过，框选/OCR/覆盖层尚未实现，Release 产物不可用 |

## 架构

8-crate workspace：

```
selinga-capture     GDI 区域截屏（物理像素，DPI 感知）
selinga-ocr         OCR 后端抽象：Windows.Media.Ocr 封装（反色/对比度/二值化/拆字重试/双通道增强）+ VLM 视觉转录
selinga-translate   Translator trait + TranSmart/Google/MyMemory/VLM 后端 + LRU 缓存 + 降级链
selinga-pipeline    OCR 行与译文的结构化配对
selinga-platform    截屏自我排除（WDA_EXCLUDEFROMCAPTURE）+ 主屏尺寸查询
selinga-app         egui 覆盖层主程序（双视口：全屏框选窗 + 方框窗；托盘/热键/触发调度/设置持久化）
selinga-cli         调试 CLI
```

## 开发

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

推送 `main` 自动跑 CI（三平台 fmt/clippy/test/build）；推 `v*` tag 自动构建三平台产物并发布到 Releases。

## License

[MIT](LICENSE)
