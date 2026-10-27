## 功能

- **启动框选**：程序打开后屏幕定格，拖出矩形即可在该位置生成翻译方框，Esc/右键取消则用默认大小
- **拖动翻译**：方框拖到哪翻译哪部分内容；内容变化自动重新识别，静止内容走帧哈希 + LRU 双层缓存
- **离线OCR**：Windows.Media.Ocr 自动识别
- **后端翻译链**：腾讯 TranSmart → Google → MyMemory，自动降级
- **AI 增强**：方框工具栏的 AI 按钮，点击--视觉大模型**识图 + 翻译**
- **双语对照 / 纯译文**两种显示模式，字号自适应
- **系统托盘 + 全局热键**：方框不在焦点也能控制
- **设置持久化**：目标语言、显示模式、快捷键保存至 TOML
- **单实例**：重复启动只会激活已运行的窗口
- **零依赖分发**：单文件可执行程序，无需安装运行时


## 快捷键与托盘

| 操作 | 热键 | 托盘菜单 |
|---|---|---|
| 隐藏 / 显示方框 | `Ctrl+Alt+H` | 隐藏方框 |
| 暂停 / 恢复识别 | `Ctrl+Alt+P` | 暂停识别 |
| 强制重新识别 | - | 重新识别 |
| 重新框选区域 | `F2` | 重新框选 |
| 打开配置文件 | - | 配置文件 |
| 退出 | `Alt+F4` | 退出 |

> 三个热键都可在 `settings.toml` 的 `[hotkeys]` 里改，保存后 1 秒内生效；
> 若某个键被别的程序占用或写法不合法，只有该热键失效，托盘与其余热键照常工作。

方框悬停时右缘出现图标工具栏：切换显示模式、复制结果、目标语言、重新识别、**AI 翻译**、暂停/恢复。AI 按钮在 `[vlm]` 未配置时变暗。


## 安装

从 [Releases](../../releases) 下载对应平台的单文件程序。

从源码构建：

```bash
git clone https://github.com/Sunse666/Selinga.git
cd Selinga
cargo build --release -p selinga-app
```

要求 Rust 1.97.1（见 `rust-toolchain.toml`）。


## 设置

配置文件位于 `%APPDATA%\selinga\settings.toml`（Windows）或 `~/.config/selinga/settings.toml`（Linux/macOS），程序运行时自动维护。

```toml
[vlm]
enabled = true
base_url = "http://localhost:11434/v1"    # OpenAI 兼容端点，本地 Ollama 示例
api_key = ""                              # 云端服务才需要；留空 = 无鉴权
model = "qwen2.5:7b"

[hotkeys]                                 # 写法：修饰键+按键
hide = "Ctrl+Alt+H"                       # 隐藏 / 显示方框
pause = "Ctrl+Alt+P"                      # 暂停 / 恢复识别
reselect = "F2"                           # 重新框选区域
```

- 自动流程：本地 Windows OCR → TranSmart → Google → MyMemory
- 点击 **AI**：自动完成识图 + 翻译，结果覆盖显示，右下角标注 `AI`；画面内容不变会一直保留，内容变化后自动回到自动流程
- AI 会先本地跑一遍 OCR：避免模型内部降采样把小字糊掉；若模型未成功识别，则退回本地 OCR 的文本、只发文本请 AI 翻译，而不是报错
- AI 结果保留每行的位置与字号：模型按提示返回归一化 bbox；若模型只回文字，则用并行的本地 OCR 行框对齐（同数量按顺序对应，不同数量按垂直中心就近匹配），因此译文仍逐行贴在原位、字号随行高自适应
- AI 请求失败时保留原译文，只在左上角角标显示原因

### 译文后处理

翻译结果落地前会统一规整（`selinga_pipeline::normalize_translation`）：

- 含中文的行：删除全部空格
- 纯拉丁行：只把连续空格压成一个，避免 `Hello world` 被连成 `Helloworld`
- 全角标点转换为英文：`，。！？；：、（）【】《》“”…—～` 及全角字母数字 `Ａ１` 全部转 ASCII
- trim 首尾空白原文保持原样


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
| Windows | 完整功能（GDI 截屏 + Windows.Media.Ocr） |
| Linux / macOS | 尚未实现（框选/OCR/覆盖层缺失） |


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


## License

[MIT](LICENSE)
