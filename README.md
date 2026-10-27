# Selinga

**Sel**ect + **Lingo** — 启动时框选屏幕任意区域，生成一个可拖动的透明方框，内容自动离线 OCR 并翻译，译文直接覆盖显示。

## 功能

- **启动即框选**：程序打开后屏幕定格（Snipaste 式），拖出矩形即可在该位置生成翻译方框，Esc/右键取消则用默认大小；托盘"重新框选"随时可换区域
- **拖框即译**：方框拖到哪译到哪；内容变化自动重新识别，静止内容走帧哈希 + LRU 双层缓存，零重复开销
- **离线 OCR**：Windows.Media.Ocr 系统级识别（中文/英文/日文等，随系统语言包），不上传画面
- **多后端翻译链**：腾讯 TranSmart（免 key 国内直连）→ Google → MyMemory，自动降级，失败不卡 UI
- **AI 增强（可选，点击触发）**：方框工具栏的 **AI** 按钮，点一次 = 视觉大模型**识图 + 翻译**一次（任意 OpenAI 兼容端点：云端 API、硅基流动、本地 Ollama 均可）；不点就完全不走 AI，自动流程只用本地 OCR + 免费翻译链
- **双语对照 / 纯译文**两种显示模式，字号自适应，一键复制
- **系统托盘 + 全局热键**：方框不在焦点也能控制
- **设置持久化**：目标语言、显示模式、快捷键存 TOML，重启保留；**暂停状态不保留**，每次启动都是运行态
- **单实例**：重复启动只会激活已运行的窗口，不会出现两个实例抢热键
- **零依赖分发**：单文件可执行程序，无需安装运行时

## 快捷键与托盘

| 操作 | 热键 | 托盘菜单 |
|---|---|---|
| 隐藏 / 显示方框 | `Ctrl+Alt+H` | 隐藏方框 |
| 暂停 / 恢复识别 | `Ctrl+Alt+P` | 暂停识别 |
| 强制重新识别 | 方框工具栏 刷新图标 | 重新识别 |
| 重新框选区域 | `F2` | 重新框选 |
| 打开配置文件 | — | 配置文件 |
| 退出 | 方框上 `Alt+F4` | 退出 |

> 三个热键都可在 `settings.toml` 的 `[hotkeys]` 里改（见下文「设置」），保存后 1 秒内生效；
> 若某个键被别的程序占用或写法不合法，只有该热键失效，托盘与其余热键照常工作。

方框悬停时右缘出现图标工具栏：切换显示模式、复制结果、目标语言（EN/ZH/JA/KO 文字代码）、重新识别、**AI（识图 + 翻译，点击一次）**、暂停/恢复；悬停图标有功能提示。AI 按钮在 `[vlm]` 未配置时变暗，点击会在角标提示去「配置文件」填写。

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

配置文件位于 `%APPDATA%\selinga\settings.toml`（Windows）或 `~/.config/selinga/settings.toml`（Linux/macOS），程序运行时自动维护。

**托盘右键 → 「配置文件」** 会用系统默认程序打开它（`.toml` 没有关联程序时自动退回记事本；想固定用某个编辑器可设环境变量 `SELINGA_EDITOR`），首次点击时自动生成含全部字段的文件，填好 AI 识别端点与 key 即可：

```toml
[vlm]
enabled = true
base_url = "http://localhost:11434/v1"   # OpenAI 兼容端点，本地 Ollama 示例
api_key = ""                              # 云端服务才需要；留空 = 无鉴权
model = "qwen2.5:7b"

[hotkeys]                                  # 写法：修饰键+按键，或单键
hide = "Ctrl+Alt+H"                       # 隐藏 / 显示方框
pause = "Ctrl+Alt+P"                      # 暂停 / 恢复识别
reselect = "F2"                           # 重新框选区域
```

> **隐私提示**：本地 OCR 不上传画面；但译文请求会把**识别出的文本**发送到腾讯 TranSmart /
> Google / MyMemory 的公共接口。启用 `[vlm]` 后，**框选区域的截图会以 PNG 形式上上传**到
> 你配置的端点——若内容敏感，请指向本地 Ollama 等自建服务。

**保存后约 1 秒内生效，无需重启**（程序每秒检测文件修改并热加载；改动与内存一致时不会重跑识别）。
改 `[hotkeys]` 会重新注册全局热键，托盘菜单里的按键提示也会同步更新。

`[vlm]` 只对**方框工具栏的 AI 按钮**生效，不影响自动流程：

- 自动流程：本地 Windows OCR → TranSmart → Google → MyMemory（**永不上传截图**）
- 点一次 **AI**：**一次请求**完成识图 + 翻译（**截图** + 目标语言一起发，模型一次返回每行「原文 / 译文 / 框」），结果覆盖显示，右下角标注 `AI`；画面内容不变会一直保留，内容变化后自动回到自动流程
- AI 会先本地跑一遍 OCR：**字很小（中位行高 < 20px）时把截图放大 2–3 倍再发**（上限长边 1568px），避免模型内部降采样把小字糊掉；若模型什么都没识别出来，则退回本地 OCR 的文本、只发文本请 AI 翻译，而不是报错
- AI 结果**保留每行的位置与字号**：模型按提示返回归一化 bbox；若模型只回文字，则用并行的本地 OCR 行框对齐（同数量按顺序对应，不同数量按垂直中心就近匹配），因此译文仍逐行贴在原位、字号随行高自适应
- AI 请求失败时保留原译文，只在左上角角标显示原因（不会把中文原文当译文覆盖）

### 译文后处理

翻译结果落地前会统一规整（`selinga_pipeline::normalize_translation`）：

- **含中日文的行**：删除**全部空格**（中文不用空格分词，`你 好 ， 世 界` → `你好,世界`）
- **纯拉丁行**：只把连续空格压成一个，避免 `Hello world` 被连成 `Helloworld`
- **中文/全角标点 → 英文**：`，。！？；：、（）【】《》“”…—～` 及全角字母数字 `Ａ１` 全部转 ASCII
- 顺手 trim 首尾空白；原文（双语模式的灰色小字）保持原样

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

7-crate workspace：

```
selinga-capture     GDI 区域截屏（物理像素，DPI 感知）
selinga-ocr         OCR 后端抽象：Windows.Media.Ocr 封装（反色/对比度/二值化/拆字重试/双通道增强）+ VLM 视觉转录
selinga-translate   Translator trait + TranSmart/Google/MyMemory/VLM 后端 + LRU 缓存 + 降级链
selinga-pipeline    OCR 行与译文的结构化配对
selinga-platform    截屏自我排除（WDA_EXCLUDEFROMCAPTURE）+ 主屏尺寸查询 + 窗口几何/配置文件打开
selinga-app         egui 覆盖层主程序（双视口：全屏框选窗 + 方框窗；托盘/热键/触发调度/设置持久化与热加载）
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
