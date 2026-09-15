# AWR 树莓派版（Raspberry Pi / aarch64 预编译二进制）

> Prebuilt **Linux arm64 (aarch64)** binaries of [agent-work-runtime](https://github.com/originoneai/agent-work-runtime) for the Raspberry Pi.
> 上游 0.3.3 的预编译包只覆盖 macOS arm64/x64、Linux x64、Windows x64，**没有 Linux arm64** —— 本分支补的就是这个缺口。
>
> 分支：`raspberry-pi-arm64`

## 文件

| 文件 | 说明 |
| --- | --- |
| `awr` | 命令行（`awr init` / `status` / `ready` / `context compile` …） |
| `awr-mcp` | MCP 服务端：stdio（单项目）或 `--listen` 常驻 HTTP（多项目共享，推荐） |
| `SHA256SUMS` | 校验值（`sha256sum -c SHA256SUMS`） |

## 安装

```bash
sudo install -m 755 awr awr-mcp /usr/local/bin/
awr --version        # -> awr 0.3.3
awr-mcp --help
```

## 构建环境（可复现）

| 项 | 值 |
| --- | --- |
| 硬件 | Raspberry Pi 5（aarch64，4 核） |
| 系统 | Debian 13 trixie（64 位，glibc 2.41） |
| 源码 | 本仓库 `main`，提交 `f853aa9`（Release 0.3.3），**未改任何代码** |
| Rust | 1.93.1（由 `rust-toolchain.toml` 指定，rustup 安装） |

```bash
cargo build --release --locked -p awr-cli -p awr-mcp
strip target/release/awr target/release/awr-mcp     # 45MB -> 36MB
```

Pi 5 上整轮编译约 22 分钟。

## 常驻共享服务（推荐用法）

HTTP 模式下 `awr-mcp` 用**一个进程服务多个项目、多个客户端**，按客户端发 Bearer 令牌授权：

```bash
awr-mcp --registry /etc/awr/service.toml --listen 172.12.0.1:8787
```

`service.toml` 声明项目与客户端：

```toml
version = 1
allowed_hosts = ["172.12.0.1", "127.0.0.1", "localhost"]

[[projects]]
key = "demo"
root = "/home/wdf-pai/awr/projects/demo"
project_id = "<awr status --json 里的 project_id>"

[[clients]]
id = "agent-a"
token_env = "AWR_TOKEN_A"     # 环境变量里放 ≥32 字符令牌
write = ["demo"]
```

端点走 Streamable HTTP：`http://<host>:8787/mcp`。**HTTP 模式没有「当前项目」概念，每次工具调用都要显式带 `project` 参数。**

## 已知限制（0.3.3）

- `awr checkout` / `awr claim` 返回 `Unsupported: operation is not implemented`
- 二进制动态链接 glibc，适用于 64 位 Raspberry Pi OS / Debian；32 位系统不适用

## 许可证

编译产物来自上游源码，许可证沿用 **Apache-2.0**（见仓库根目录 `LICENSE`）。本分支仅做编译与打包，与上游项目无隶属关系。
