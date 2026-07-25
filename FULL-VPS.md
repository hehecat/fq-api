# 番茄 Full API（VPS）

公网： **https://fq-full.oyufen.com**  
用途：App `full` + unidbg 签名，获取**完整章节**（绕过网页试读锁）。

## 架构

```text
阅读 / curl
    │
    ▼
https://fq-full.oyufen.com          (Cloudflare Tunnel)
    │
    ▼
127.0.0.1:18081  fq-full-adapter    (Python，补全绝对 URL + HTML 正文)
    │
    ▼
127.0.0.1:18080  unidbg-fq 容器     (gxmandppx/unidbg-fq:latest)
    │
    ▼
番茄 App 接口（签名后 full / search / toc / book）
```

## 接口（适配层）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/health` | 健康检查 |
| GET | `/search?key=&page=&size=` | 搜索（page≥1，size≤10） |
| GET | `/book/{bookId}` | 详情（含 `toc_url`） |
| GET | `/toc/{bookId}` | 目录（`chapters[].url` 绝对地址） |
| GET | `/chapter/{bookId}/{chapterId}` | 全文 HTML（`data.content`） |

也兼容：

- `/api/search?q=`
- `/api/book/{id}`
- `/api/book/{id}/directory`
- `/api/book/{id}/chapter/{item_id}`

### 示例

```bash
curl 'https://fq-full.oyufen.com/health'
curl 'https://fq-full.oyufen.com/search?key=我不是戏神&page=1&size=5'
curl 'https://fq-full.oyufen.com/book/7276384138653862966'
curl 'https://fq-full.oyufen.com/toc/7276384138653862966'
curl 'https://fq-full.oyufen.com/chapter/7276384138653862966/7283421685154480674'
```

第 30 章（网页常锁试读）实测 `word_number ≈ 2226`，`source=app_full+unidbg`。

## 服务器维护

```bash
ssh racknerd-cf

# 签名容器
docker ps --filter name=fqnovel
docker logs -f fqnovel
docker restart fqnovel

# 适配层
systemctl status fq-full-adapter
journalctl -u fq-full-adapter -f
systemctl restart fq-full-adapter

# Tunnel
systemctl status cloudflared
cat /etc/cloudflared/config.yml
```

路径：

| 路径 | 内容 |
|---|---|
| `/opt/fq/api/adapter.py` | 适配层 |
| `/etc/systemd/system/fq-full-adapter.service` | systemd |
| `/etc/cloudflared/config.yml` | tunnel：`fq-full.oyufen.com` → `:18081` |

## 阅读书源

[`legado-booksource-full.json`](./legado-booksource-full.json)

## 注意

- 依赖签名服务与上游风控，可能限流
- 控制并发；仅供学习研究
- 与 Worker 网页源不同：Worker 只能 SSR 试读
