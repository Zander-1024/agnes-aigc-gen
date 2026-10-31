# Agnes Image 2.5 Flash

核对日期：2026-10-31。依据：[官方接口文档](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-image-25-flash)。本文记录本项目使用的接入方式。

`POST https://apihub.agnes-ai.com/v1/images/generations`，使用 `Authorization: Bearer YOUR_API_KEY`。

| 字段 | 用法 |
|------|------|
| `model` | `agnes-image-2.5-flash` |
| `prompt` | 生成或编辑指令，必填 |
| `size` | 必填档位：`1K`、`2K`、`3K`、`4K`；CLI 默认 `1K` |
| `ratio` | `1:1`、`3:4`、`4:3`、`16:9`、`9:16`、`2:3`、`3:2`、`21:9` |
| `extra_body.image` | 编辑／合成的参考图片数组；公共 URL 或 Data URI |
| `extra_body.response_format` | CLI 请求 `url`；接口也支持 `b64_json` |

参考图片可直接传 `data:image/png;base64,...` 或 `data:image/jpeg;base64,...`。直接调用 API 时使用完整 Data URI；CLI 的 `image -i` 还接受裸 Base64，并负责转换。

官网参数表也列出顶层 `image`；本项目采用详细说明和编辑示例中的 `extra_body.image` 写法。`response_format` 放在 `extra_body` 内。

```json
{
  "model": "agnes-image-2.5-flash",
  "prompt": "保留产品轮廓，将背景改为浅色摄影棚",
  "size": "2K",
  "ratio": "4:3",
  "extra_body": {
    "image": ["https://example.com/product.png"],
    "response_format": "url"
  }
}
```

读取 `data[0].url`；Base64 结果读取 `data[0].b64_json`。CLI 默认输出远程 URL，`--save` 才下载；本地图片和内联图片由 CLI 转为 Data URI。`-n 2–4` 是多个独立请求。

```bash
agnes-aigc-gen image -p "海边的玻璃建筑，柔和晨光" --ratio 16:9 --size 2K
agnes-aigc-gen image -p "改为水彩画风" -i ./photo.png --size 1K
```

尺寸档位表示模型原生尺寸，例如 `16:9` 的 `2K` 为 `2624x1472`，并非 `2560x1440`。计费和可用权限以平台为准。
