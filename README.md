# fq-api

番茄小说（fanqienovel）**Rust Web API**：搜索 / 简介 / 目录 / 章节明文。

默认走 **官网 Web 链路**，**不依赖手机 / 模拟器 / unidbg**。  
章节正文通过阅读页 SSR + 预生成字体映射表还原；也支持本地加密缓存 AES 解密、可选签名 sidecar。

> 仅供学习与研究逆向工程技术。请遵守当地法律与目标站点服务条款，勿用于未授权的批量抓取或商业用途。

## 功能

| 接口 | 说明 |
|---|---|
| `GET /health` | 健康检查 / 密钥与字体目录状态 |
| `GET /api/search?q=&page=0&size=10` | 搜索（`x-tt-zhal` 字体表解码） |
| `GET /api/book/{book_id}` | 书籍详情 / 简介 |
| `GET /api/book/{book_id}/directory` | 目录 |
| `GET /api/book/{book_id}/chapter/{item_id}` | 章节纯文本 |
| `GET /api/book/{book_id}/chapter/{item_id}/raw` | 章节文本（可含 HTML） |

在线示例（Cloudflare Worker 同源能力）：

- https://fq-api.hehecat.workers.dev/health

## 快速开始

### 依赖

- Rust 1.75+（edition 2021）
- 可选：Python 3 + `fonttools pillow numpy freetype-py`（仅生成新字体映射时）

### 构建运行

```bash
git clone https://github.com/hehecat/fq-api.git
cd fq-api

# 可选：AES 密钥（仅本地加密章节缓存需要）
cp data/crypt_keys.example.json data/crypt_keys.json

cargo build --release
./run.sh
# 或
FQ_ENABLE_ADB=false PORT=18080 ./target/release/fq-api
```

服务默认监听 `http://0.0.0.0:18080`。

### 试一下

```bash
# 搜索
curl 'http://127.0.0.1:18080/api/search?q=我不是戏神&size=5'

# 详情
curl 'http://127.0.0.1:18080/api/book/7276384138653862966'

# 目录
curl 'http://127.0.0.1:18080/api/book/7276384138653862966/directory'

# 第一章
curl 'http://127.0.0.1:18080/api/book/7276384138653862966/chapter/7276663560427471412'
```

## 数据源策略

```
搜索 / 简介 / 目录  ──►  fanqienovel.com Web API / 页面
章节正文（默认）    ──►  /reader/{itemId} SSR + 字体映射表查表
章节正文（可选）    ──►  本地 AES 缓存 / FQ_SIGNER_URL App full / ADB
```

1. **搜索**：读响应头 `x-tt-zhal`（`f=<font_id>`），用 `font_maps/{font_id}.json` 做 PUA→汉字映射。  
2. **章节**：解析阅读页 `awesome-font/c/{font_id}`，同样查表解码，并写入 `data/chapters/plaintext/` 缓存。  
3. **字体映射是离线生成的**；运行时只做 `HashMap` 查表（O(n)），**不会**实时跑 FreeType。

内置映射：

- `font_maps/c207f68a84deae3.json` — 搜索列表
- `font_maps/dc027189e0ba4cd.json` — 阅读页正文

### 生成 / 更新字体映射

字体 ID 会变。新 ID 出现时：

```bash
pip install fonttools pillow numpy freetype-py
python3 tools/build_font_map.py --from-search '我不是戏神'
# 或
python3 tools/build_font_map.py --font-id <f字段>
```

## 章节 AES 解密（可选）

用于解密 App 本地 / `full` 接口拿到的加密 `content`：

```
content (base64)
  -> AES-128-CBC   key=16B(hex32), iv=前16字节
  -> PKCS7 unpad
  -> GZIP
  -> HTML / 纯文本
```

`registerkey` 外壳密钥（研究用）：

```text
REG_KEY = ac25c67ddd8f38c1b37a2348828e222e
```

`data/crypt_keys.json` 示例：

```json
{
  "208700406": "9A1AF690605DDC2F556388A3A6B29744"
}
```

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `PORT` | `18080` | 监听端口 |
| `FQ_KEY_FILE` | `data/crypt_keys.json` | `key_version -> AES key` |
| `FQ_AES_KEY` / `FQ_KEY_VERSION` | - | 单密钥覆盖 |
| `FQ_FONT_MAP_DIR` | `font_maps` | 字体映射目录 |
| `FQ_CHAPTER_CACHE` | `data/chapters/decoded_json` | 加密章节 JSON 缓存 |
| `FQ_PLAINTEXT_DIR` | `data/chapters/plaintext` | 已解码明文缓存 |
| `FQ_ENABLE_ADB` | `false` | 是否允许 adb 兜底 |
| `FQ_ADB_SERIAL` | `127.0.0.1:16384` | adb 设备 |
| `FQ_SIGNER_URL` | - | 签名 sidecar（unidbg 等） |
| `FQ_API_BASE` | `https://api5-normal-sinfonlineb.fqnovel.com` | App API 基址 |

## 目录结构

```text
fq-api/
  src/
    main.rs            # HTTP 路由
    crypto.rs          # AES-CBC + GZIP / registerkey
    fontmap.rs         # x-tt-zhal / 阅读页 PUA 映射
    models.rs
    state.rs
    services/
      web.rs           # 搜索 / 详情 / 目录 / SSR
      chapter.rs       # 章节聚合 + 缓存
  font_maps/           # 预生成字体映射（提交进仓库）
  tools/
    build_font_map.py  # 离线生成映射
  data/
    crypt_keys.example.json
    chapters/          # 运行时缓存（默认不提交正文）
  run.sh
  Cargo.toml
```

## 部署

### VPS / 本机

```bash
cargo build --release
FQ_ENABLE_ADB=false PORT=18080 ./target/release/fq-api
```

systemd / docker 自行挂载 `font_maps/` 与可选 `data/`。

### Cloudflare Workers

同能力的轻量版在姊妹项目思路下可部署到 Workers（纯 Web + 内置映射，无 ADB/so）。  
本仓库是 **完整 Rust 服务**，适合 VPS；Workers 版源码见本机 `fq-worker/`（若一并维护）。

## 批量说明

- **适合**：按需 API、中小批量、依赖明文缓存复用。  
- **不太适合**：无节制高并发全站爬取（阅读页 HTML 更重，易触发上游限流）。  
- 大批量可考虑：`FQ_SIGNER_URL` + App `full`/`batch_full`，或先用本服务灌 `plaintext` 缓存。

## License

MIT（代码）。目标站点内容版权归原作者与平台所有；逆向研究请自担合规责任。
