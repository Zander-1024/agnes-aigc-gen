# Agnes Video 2.5 Flash

核对日期：2026-10-31。依据：[官方 Flash 文档](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-25-flash)。通用字段和异步流程见 [Video 2.5](agnes-video-2.5.md)。

| 项目 | Flash |
|------|-------|
| 模型 | `agnes-video-2.5-flash` |
| `size` | 仅 `720P` |
| `seconds` | 字符串整数 `"4"`–`"12"` |
| 模式 | `text`、`keyframe`、`reference` |
| 参考图片 | 最多 5 张 |
| 参考音频 | 最多 3 段 |
| 参考视频 | 不支持 |
| 查询 | `/agnesapi?video_id=<ID>&model_name=agnes-video-2.5-flash` |
| 结果 | `status=completed` 时读取顶层 `url` |

CLI 兼容不带 `/v1` 和末尾带 `/v1` 的 `base_url`，全部模式统一向同一网关的 `/agnesapi` 查询，并保留创建时的模型名。Flash 的像素映射与普通版不同；例如官方记录的 `16:9` 输出为 `1280x704`，实际尺寸以文件为准。

首尾帧及参考图输入可用 HTTPS URL、`asset://`、本地 PNG/JPEG/WebP、Base64 或图片 Data URI；本地与内联内容验证后转换为 JPEG Data URI，远程 URL 保持原样。音频参考仍要求 HTTPS URL。Data URI 的测试网关、日期和已验证范围见 [Video 2.5](agnes-video-2.5.md)，不将测试结果视为官网承诺。

```bash
agnes-aigc-gen video --model agnes-video-2.5-flash -p "海浪轻轻拍打沙滩，环境声自然" -d 5 --size 720P
agnes-aigc-gen video --model agnes-video-2.5-flash -p "从首帧自然移动到尾帧" --first-frame ./start.png --last-frame ./end.jpg -d 4
agnes-aigc-gen video --model agnes-video-2.5-flash -p "参考 <Picture 1> 的角色，跟随 <Audio 1> 舞动" --mode reference -i https://example.com/character.png --audio https://example.com/beat.mp3 --async
```

也可通过 `config set video-model agnes-video-2.5-flash` 设置默认。价格和体验政策可能变化，请以平台公告为准。
