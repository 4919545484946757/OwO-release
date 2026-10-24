//! 主流 OpenAI 兼容提供商的静态预设（地址 / 代表模型 / 多模态能力）。
//!
//! 目标：设置页选提供商 → 只填 API Key 即可用；也支持完全自定义 base_url/model。
//! 这里只放**公开的端点与模型名**，不含任何凭据；凭据仍走环境变量或本地加密配置。

use serde::{Deserialize, Serialize};

/// 多模态能力位：前端据此决定"能否传图 / 能否生成图 / 能否合成语音"。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalitySupport {
    /// 文本入、文本出（所有对话模型都有）。
    pub text: bool,
    /// 图片输入（视觉理解）。
    pub image_input: bool,
    /// 图片输出（文生图）。
    pub image_output: bool,
    /// 音频输入（语音识别/理解）。
    pub audio_input: bool,
    /// 音频输出（语音合成）。
    pub audio_output: bool,
}

impl ModalitySupport {
    const TEXT: Self = Self {
        text: true,
        image_input: false,
        image_output: false,
        audio_input: false,
        audio_output: false,
    };

    const VISION: Self = Self {
        text: true,
        image_input: true,
        image_output: false,
        audio_input: false,
        audio_output: false,
    };
}

/// 一个提供商预设。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderPreset {
    /// 稳定 id（前端/配置里存它）。
    pub id: &'static str,
    /// 展示名。
    pub name: &'static str,
    /// OpenAI 兼容 base_url（已含版本段，调用时拼 `/chat/completions`）。
    pub base_url: &'static str,
    /// 该提供商是否接受空 API Key（本地部署）。
    pub allows_empty_key: bool,
    /// 推荐对话模型（第一个为默认）。
    pub models: &'static [&'static str],
    /// 对话模型的多模态能力。
    pub chat_modalities: ModalitySupport,
    /// 专用的图像生成模型（为空表示该提供商不走本 SDK 的文生图通道）。
    pub image_models: &'static [&'static str],
    /// 专用的语音合成模型。
    pub speech_models: &'static [&'static str],
    /// 设置页提示（计费/区域/兼容性注意点）。
    pub note: &'static str,
}

/// 内置预设表。按"国内可直连 + 主流国际"排序；新增提供商只需加一行。
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "bigmodel",
        name: "智谱 BigModel（GLM）",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        allows_empty_key: false,
        models: &["glm-5.3-flash", "glm-4.6", "glm-4.5-air", "glm-4v-plus"],
        chat_modalities: ProviderPreset::vision(),
        image_models: &["cogview-4"],
        speech_models: &["cogtts"],
        note: "GLM-5 系列始终思考，可用 OWO_REASONING_EFFORT=low 降低思考量。",
    },
    ProviderPreset {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        allows_empty_key: false,
        models: &["gpt-4.1", "gpt-4.1-mini", "gpt-4o", "o4-mini"],
        chat_modalities: ProviderPreset::vision(),
        image_models: &["gpt-image-1", "dall-e-3"],
        speech_models: &["tts-1", "gpt-4o-mini-tts"],
        note: "国内直连不稳定时设置 OWO_HTTP_PROXY。",
    },
    ProviderPreset {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        allows_empty_key: false,
        models: &["deepseek-chat", "deepseek-reasoner"],
        chat_modalities: ModalitySupport::TEXT,
        image_models: &[],
        speech_models: &[],
        note: "纯文本模型，不支持图片输入。",
    },
    ProviderPreset {
        id: "moonshot",
        name: "Moonshot（Kimi）",
        base_url: "https://api.moonshot.cn/v1",
        allows_empty_key: false,
        models: &[
            "kimi-k2-0905-preview",
            "moonshot-v1-128k",
            "moonshot-v1-32k-vision-preview",
        ],
        chat_modalities: ProviderPreset::vision(),
        image_models: &[],
        speech_models: &[],
        note: "长上下文见长；视觉模型名为 moonshot-v1-*-vision-preview。",
    },
    ProviderPreset {
        id: "dashscope",
        name: "阿里云百炼（Qwen）",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        allows_empty_key: false,
        models: &["qwen3-max", "qwen-plus", "qwen-vl-max", "qwen3-coder-plus"],
        chat_modalities: ProviderPreset::vision(),
        image_models: &["wanx2.1-t2i-turbo"],
        speech_models: &["cosyvoice-v2"],
        note: "OpenAI 兼容模式端点，模型名带 qwen-vl 才支持图片输入。",
    },
    ProviderPreset {
        id: "siliconflow",
        name: "SiliconFlow（硅基流动）",
        base_url: "https://api.siliconflow.cn/v1",
        allows_empty_key: false,
        models: &[
            "deepseek-ai/DeepSeek-V3.2",
            "Qwen/Qwen3-235B-A22B-Instruct",
            "Qwen/Qwen2.5-VL-72B-Instruct",
        ],
        chat_modalities: ProviderPreset::vision(),
        image_models: &["Kwai-Kolors/Kolors"],
        speech_models: &["FunAudioLLM/CosyVoice2-0.5B"],
        note: "聚合平台，模型名带厂商前缀（org/model）。",
    },
    ProviderPreset {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        allows_empty_key: false,
        models: &[
            "anthropic/claude-sonnet-4.5",
            "google/gemini-2.5-pro",
            "openai/gpt-4.1",
            "deepseek/deepseek-chat",
        ],
        chat_modalities: ProviderPreset::vision(),
        image_models: &[],
        speech_models: &[],
        note: "一个 Key 路由多家模型；计费按模型单价，注意加 OWO_HTTP_PROXY。",
    },
    ProviderPreset {
        id: "ollama",
        name: "Ollama（本地）",
        base_url: "http://127.0.0.1:11434/v1",
        allows_empty_key: true,
        models: &["qwen3:8b", "llama3.2:3b", "llava:7b"],
        chat_modalities: ProviderPreset::vision(),
        image_models: &[],
        speech_models: &[],
        note: "本地推理，地址可改为局域网机器；允许空 Key。",
    },
    ProviderPreset {
        id: "vllm",
        name: "vLLM / LM Studio（自建）",
        base_url: "http://127.0.0.1:8000/v1",
        allows_empty_key: true,
        models: &["local-model"],
        chat_modalities: ModalitySupport::TEXT,
        image_models: &[],
        speech_models: &[],
        note: "自建 OpenAI 兼容服务；模型名以服务端 /v1/models 为准。",
    },
    ProviderPreset {
        id: "custom",
        name: "自定义（OpenAI 兼容）",
        base_url: "",
        allows_empty_key: true,
        models: &[],
        chat_modalities: ModalitySupport::TEXT,
        image_models: &[],
        speech_models: &[],
        note: "任意 OpenAI 兼容端点：填 base_url + model + api_key 即可。",
    },
];

impl ProviderPreset {
    const fn vision() -> ModalitySupport {
        ModalitySupport::VISION
    }

    pub fn by_id(id: &str) -> Option<&'static ProviderPreset> {
        let needle = id.trim().to_ascii_lowercase();
        PROVIDER_PRESETS.iter().find(|preset| preset.id == needle)
    }

    /// 按 base_url 反查预设（用户直接填地址时也能识别提供商与多模态能力）。
    pub fn detect_by_base_url(base_url: &str) -> Option<&'static ProviderPreset> {
        let lower = base_url.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return None;
        }
        PROVIDER_PRESETS
            .iter()
            .filter(|preset| !preset.base_url.is_empty())
            .find(|preset| {
                lower.starts_with(preset.base_url)
                    || preset
                        .base_url
                        .split("://")
                        .nth(1)
                        .and_then(|rest| rest.split('/').next())
                        .is_some_and(|host| lower.contains(host))
            })
    }

    /// 该提供商默认模型（自定义/空表返回 None）。
    pub fn default_model(&self) -> Option<&'static str> {
        self.models.first().copied()
    }

    /// 该提供商是否具备文生图 / 语音合成能力。
    pub fn supports_image_generation(&self) -> bool {
        !self.image_models.is_empty()
    }

    pub fn supports_speech(&self) -> bool {
        !self.speech_models.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_unique_ids_and_valid_urls() {
        let mut seen = std::collections::HashSet::new();
        for preset in PROVIDER_PRESETS {
            assert!(seen.insert(preset.id), "重复 id: {}", preset.id);
            if preset.id == "custom" {
                assert!(preset.base_url.is_empty());
            } else {
                assert!(
                    preset.base_url.starts_with("http"),
                    "{} 的 base_url 非法: {}",
                    preset.id,
                    preset.base_url
                );
                assert!(preset.default_model().is_some(), "{} 缺少模型", preset.id);
            }
            assert!(preset.chat_modalities.text, "{} 必须支持文本", preset.id);
        }
    }

    #[test]
    fn detect_by_base_url_matches_hosts() {
        assert_eq!(
            ProviderPreset::detect_by_base_url("https://api.deepseek.com/v1").map(|p| p.id),
            Some("deepseek")
        );
        assert_eq!(
            ProviderPreset::detect_by_base_url("http://127.0.0.1:11434/v1").map(|p| p.id),
            Some("ollama")
        );
        assert!(ProviderPreset::detect_by_base_url("").is_none());
    }

    #[test]
    fn multimodal_capabilities_are_declared() {
        let bigmodel = ProviderPreset::by_id("bigmodel").expect("bigmodel");
        assert!(bigmodel.chat_modalities.image_input);
        assert!(bigmodel.supports_image_generation());
        assert!(bigmodel.supports_speech());
        let deepseek = ProviderPreset::by_id("deepseek").expect("deepseek");
        assert!(!deepseek.chat_modalities.image_input);
        assert!(!deepseek.supports_image_generation());
    }
}
