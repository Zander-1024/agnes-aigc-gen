# Agnes Video 2.5

核对日期：2026-10-31。依据：[官方接口文档](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-25)。Flash 的差异见 [2.5 Flash](agnes-video-2.5-flash.md)。

| 操作 | 接口 |
|------|------|
| 创建 | `POST https://apihub.agnes-ai.com/v1/videos` |
| 查询 | `GET https://apihub.agnes-ai.com/agnesapi?video_id=<ID>&model_name=agnes-video-2.5` |

请求使用 Bearer API Key。创建后保存 `video_id`，查询时同时传入创建任务所用的模型名称。

| 字段 | 用法 |
|------|------|
| `model` | `agnes-video-2.5` |
| `prompt` | 必填；参考素材可用 `<Picture N>`、`<Audio N>`、`<Video N>` 指代，各类从 1 编号 |
| `mode` | 必填：`text`、`keyframe`、`reference` |
| `seconds` | 字符串整数 `"4"`–`"12"`；默认 `"5"` |
| `size` | `720P`、`1080P`、`1K`、`2K` |
| `aspect_ratio` | `21:9`、`16:9`、`4:3`、`1:1`、`3:4`、`9:16` |
| `seed` | 可选整数 |
| `n` | 仅支持 `1` |

| 模式 | 媒体字段 |
|------|----------|
| `text` | 无媒体 |
| `keyframe` | `first_frame`／`last_frame` 至少一个；不与参考数组混用 |
| `reference` | `images`、`audios`、`videos` 至少一类非空；不与首尾帧混用 |

媒体必须为服务可访问的公共 URL。`videos` 中的对象包含 `url`，可选非负 `start_seconds` 和布尔 `require_audio`。图片最多 8 张，音频最多 3 段，视频最多 1 个，总文件数最多 12。图片单张 <15 MB、总计 <50 MB、宽高各 256–5760；音频单段 <15 MB、总计 <64 MB、总时长 2–12 秒；视频 <50 MB、2–12 秒、24–60 FPS。CLI 校验模式与数量，文件规格由服务校验。

```json
{
  "model": "agnes-video-2.5",
  "prompt": "根据 <Picture 1> 的角色设计和 <Audio 1> 的节奏生成舞蹈",
  "mode": "reference",
  "seconds": "5",
  "size": "1080P",
  "aspect_ratio": "9:16",
  "images": ["https://example.com/character.png"],
  "audios": ["https://example.com/beat.mp3"]
}
```

请求直接发送上述顶层字段。Python OpenAI SDK 的 `extra_body` 是合并到顶层的 SDK 选项，不能照搬为视频请求的嵌套对象。

读取顶层 `status` 与 `progress`；`completed` 后从 `url` 取结果。`remixed_from_video_id` 表示来源视频 ID，不是结果链接；`internal_status` 不决定是否完成。失败时读取 `error`。

```bash
agnes-aigc-gen video -p "云海上缓慢前进的列车" -d 5 --size 1080P --async
agnes-aigc-gen video -p "从清晨过渡到傍晚" --first-frame https://example.com/start.png --last-frame https://example.com/end.png
agnes-aigc-gen video -p "跟随 <Video 1> 的动作" --video '{"url":"https://example.com/motion.mp4","start_seconds":2,"require_audio":false}'
agnes-aigc-gen task wait 3
```

CLI 省略模式时：无媒体为 `text`，单张 `-i` 为首帧 `keyframe`，多张 `-i` 或音视频为 `reference`。`--mode reference -i URL` 可将单图用作参考。输出画幅由 `--ratio` 指定，不要求参考图片比例相同。`1K` 固定输出 `1024x1024`，其他档位按官方画幅映射；具体尺寸以生成文件为准。

当前接口不配置帧率、帧数、推理步数或自定义宽高；CLI 移除旧版帧率与负向提示参数。现有配置切换方法见 [SETUP.md](../SETUP.md#upgrading-model-settings)。
