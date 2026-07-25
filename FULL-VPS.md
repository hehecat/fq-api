# 番茄 Full API（VPS 部署）

部署主机：`192.129.159.236`（racknerd）  
公网入口：`https://fq-full.oyufen.com`（Cloudflare Tunnel）  
容器：`gxmandppx/unidbg-fq:latest`（签名 + App full 解密）

## 接口

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/search?key=&page=&size=` | 搜索（page 从 1） |
| GET | `/book/{bookId}` | 详情 |
| GET | `/toc/{bookId}` | 目录 `data.item_data_list` |
| GET | `/chapter/{bookId}/{chapterId}` | **全文** `data.txtContent` |

### 示例

```bash
curl 'https://fq-full.oyufen.com/search?key=我不是戏神&page=1&size=5'
curl 'https://fq-full.oyufen.com/book/7276384138653862966'
curl 'https://fq-full.oyufen.com/toc/7276384138653862966'
# 网页会锁的第30章，这里是全文
curl 'https://fq-full.oyufen.com/chapter/7276384138653862966/7283421685154480674'
```

实测：第30章 `txtContent` 长度约 2226 字（网页 SSR 仅试读约 155 字）。

## 服务器维护

```bash
ssh racknerd-cf
docker ps --filter name=fqnovel
docker logs -f fqnovel
docker restart fqnovel

# 仅本机端口（经 tunnel 对外）
# 0.0.0.0:18080 -> container 9999
```

cloudflared 配置：`/etc/cloudflared/config.yml`

```yaml
- hostname: fq-full.oyufen.com
  service: http://127.0.0.1:18080
```

## 阅读书源

见 `legado-booksource-full.json`。

导入后搜索 `我不是戏神`，打开后半章节应不再是试读截断。

## 注意

- 依赖 unidbg 签名与上游 App 接口，可能有限流/风控
- 请控制并发；仅供学习研究
- 与 Cloudflare Worker 网页源（`fq-api.hehecat.workers.dev`）不同：Worker 只能网页试读，本服务走 App full
