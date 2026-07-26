# fq-api

番茄小说相关 **API / 阅读书源** 工具集（学习研究用）。

> **仅供学习与研究。** 请遵守当地法律与目标站点服务条款；勿用于未授权批量抓取或商业用途。  
> 仓库已设为 **Private**。

## 两套线上入口

| 入口 | 地址 | 正文能力 | 说明 |
|---|---|---|---|
| **Full（推荐阅读）** | https://fq-full.oyufen.com | **App full + 签名解密，完整章节** | VPS `unidbg-fq` + 适配层 |
| Worker（网页） | https://fq-api.hehecat.workers.dev | 官网 SSR + 字体表，**可能试读截断** | Cloudflare Workers |

### Full API（完整正文）

```bash
# 健康检查
curl 'https://fq-full.oyufen.com/health'

# 搜索（page 从 1，size ≤ 10）
curl 'https://fq-full.oyufen.com/search?key=我不是戏神&page=1&size=10'

# 详情
curl 'https://fq-full.oyufen.com/book/7276384138653862966'

# 目录（每章带绝对 url）
curl 'https://fq-full.oyufen.com/toc/7276384138653862966'

# 正文（HTML 段落，完整）
curl 'https://fq-full.oyufen.com/chapter/7276384138653862966/7283421685154480674'
```

兼容别名路径：

- `/api/search?q=`
- `/api/book/{id}`
- `/api/book/{id}/directory`
- `/api/book/{id}/chapter/{item_id}`

### Worker API（网页链路）

```bash
curl 'https://fq-api.hehecat.workers.dev/health'
curl 'https://fq-api.hehecat.workers.dev/api/search?q=我不是戏神&page=1&size=10'
curl 'https://fq-api.hehecat.workers.dev/api/book/7276384138653862966'
curl 'https://fq-api.hehecat.workers.dev/api/book/7276384138653862966/directory'
curl 'https://fq-api.hehecat.workers.dev/api/book/7276384138653862966/chapter/7276663560427471412'
```

注意：上游搜索 `page_count` **最大 10**；Worker 已强制 clamp。

## 阅读 / Legado 书源

适配 [legado-Sigma](https://github.com/angel888k/legado-Sigma-2026) / 原版阅读。

| 书源文件 | 用途 |
|---|---|
| [`legado-booksource-full.json`](./legado-booksource-full.json) | **完整正文（推荐）** → `fq-full.oyufen.com` |
| [`legado-booksource.json`](./legado-booksource.json) | 网页 Worker，可能试读不全 |

导入 Full 源：

```text
legado://import/bookSource?src=https://raw.githubusercontent.com/hehecat/fq-api/main/legado-booksource-full.json
```

（Private 仓库 raw 需登录/token；也可把 JSON 拷到手机本地导入。）

手机浏览器先确认：

- https://fq-full.oyufen.com/health → ok / adapter

## 本仓库：Rust `fq-api` 服务

本地/VPS 可编译运行的 Axum 服务（Web 搜索/详情/目录/SSR 章节 + 可选 AES/ADB/签名）。

### 功能

| 接口 | 说明 |
|---|---|
| `GET /health` | 健康检查 |
| `GET /api/search?q=&page=&size=` | 搜索（字体表解码） |
| `GET /api/book/{book_id}` | 详情 |
| `GET /api/book/{book_id}/directory` | 目录 |
| `GET /api/book/{book_id}/chapter/{item_id}` | 章节 |
| `GET /api/book/{book_id}/chapter/{item_id}/raw` | 章节 + HTML |

### 构建运行

```bash
git clone https://github.com/hehecat/fq-api.git
cd fq-api
cp data/crypt_keys.example.json data/crypt_keys.json   # 可选，AES 缓存用
cargo build --release
FQ_ENABLE_ADB=false PORT=18080 ./target/release/fq-api
# 或 ./run.sh
```

依赖：Rust 1.75+。生成新字体映射时可选 Python：`fonttools pillow numpy freetype-py`。

```bash
python3 tools/build_font_map.py --from-search '我不是戏神'
```

### 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `PORT` | `18080` | 监听端口 |
| `FQ_KEY_FILE` | `data/crypt_keys.json` | AES key_version 映射 |
| `FQ_FONT_MAP_DIR` | `font_maps` | 字体映射目录 |
| `FQ_CHAPTER_CACHE` | `data/chapters/decoded_json` | 加密章节缓存 |
| `FQ_PLAINTEXT_DIR` | `data/chapters/plaintext` | 明文缓存 |
| `FQ_ENABLE_ADB` | `false` | adb 兜底 |
| `FQ_SIGNER_URL` | - | 签名 sidecar（`/sign`） |
| `FQ_API_BASE` | App API 基址 | full 请求用 |

### 数据源策略（Rust 服务）

```text
搜索/简介/目录  → fanqienovel.com Web
章节默认        → 阅读页 SSR + font_maps 查表
章节可选        → 本地 AES 缓存 / FQ_SIGNER_URL full / ADB
```

字体映射 **离线生成**，运行时只做 HashMap 查表。

内置：

- `font_maps/c207f68a84deae3.json` — 搜索
- `font_maps/dc027189e0ba4cd.json` — 阅读页

AES（App 缓存 / full）：

```text
content(base64) → AES-128-CBC(iv=前16B) → PKCS7 → GZIP → HTML
```

## VPS Full 部署说明

详见 [`FULL-VPS.md`](./FULL-VPS.md)。

摘要：

| 项 | 值 |
|---|---|
| 主机 | racknerd（SSH / cloudflared） |
| 容器 | `gxmandppx/unidbg-fq:latest` → `127.0.0.1:18080` |
| 适配层 | `fq-full-adapter` → `127.0.0.1:18081` |
| 域名 | `fq-full.oyufen.com` → adapter |
| 阅读书源 | `legado-booksource-full.json` |

维护：

```bash
ssh racknerd-cf
docker ps --filter name=fqnovel
docker logs -f fqnovel
systemctl status fq-full-adapter cloudflared
```

## 目录结构

```text
fq-api/
  src/                      # Rust 服务
  font_maps/                # 预生成字体映射
  tools/build_font_map.py   # 离线生成映射
  data/                     # 运行时缓存（正文默认不提交）
  legado-booksource-full.json
  legado-booksource.json
  FULL-VPS.md
  LEGADO.md
  run.sh
```

## 常见问题

**搜索 502 / 参数有误**  
上游 `page_count` 不能大于 10。书源 `size=10`，Worker/adapter 已 clamp。

**Worker 正文不全**  
网页 `isChapterLock` 试读。请用 Full 源 `fq-full.oyufen.com`。

**阅读目录/正文失败**  
用最新 `legado-booksource-full.json`（绝对 URL 字段，无 JS 拼链接）。

**workers.dev 手机打不开**  
网络限制；Full 域名走 Cloudflare Tunnel，一般更稳。

## License

MIT（本仓库代码）。目标站点内容版权归原作者与平台；使用风险自负。
