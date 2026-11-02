use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::AppConfig;

const FILE_NAME: &str = "request_params.json";

fn empty_object() -> Value {
    json!({})
}

const fn schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RequestKind {
    Text,
    Image,
    Video,
}

impl RequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BodyDefaults {
    pub body: Value,
}

impl Default for BodyDefaults {
    fn default() -> Self {
        Self { body: empty_object() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TypeDefaults {
    #[serde(default = "empty_object")]
    pub body: Value,
    pub models: BTreeMap<String, BodyDefaults>,
}

impl Default for TypeDefaults {
    fn default() -> Self {
        Self { body: empty_object(), models: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RequestParams {
    #[serde(default = "schema_version")]
    pub version: u32,
    pub text: TypeDefaults,
    pub image: TypeDefaults,
    pub video: TypeDefaults,
}

impl Default for RequestParams {
    fn default() -> Self {
        Self {
            version: schema_version(),
            text: TypeDefaults { body: empty_object(), ..TypeDefaults::default() },
            image: TypeDefaults { body: empty_object(), ..TypeDefaults::default() },
            video: TypeDefaults { body: empty_object(), ..TypeDefaults::default() },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedRequestParams {
    pub body: Value,
    pub config_hash: String,
    pub layers: Vec<String>,
}

/// Merge objects by key; arrays, scalars, and null replace only their own value.
pub fn deep_merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                deep_merge(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}

impl RequestParams {
    pub fn path() -> Result<PathBuf> {
        Ok(AppConfig::config_dir()?.join(FILE_NAME))
    }

    pub fn load() -> Result<Self> {
        Self::load_from_path(&Self::path()?)
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
        };
        let params: Self = serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        params.validate()?;
        Ok(params)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let directory = path
            .parent()
            .context("request parameter file needs a parent directory")?;
        fs::create_dir_all(directory)?;
        let temporary = directory.join(format!(".{FILE_NAME}.{}.tmp", rand::random::<u64>()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            // std::fs::rename replaces an existing file on supported platforms.
            fs::rename(&temporary, path).with_context(|| format!("replace {}", path.display()))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == schema_version(),
            "unsupported request_params version {}; expected 1",
            self.version
        );
        for kind in [RequestKind::Text, RequestKind::Image, RequestKind::Video] {
            let defaults = self.for_kind(kind);
            validate_body(kind, &defaults.body)?;
            for (model, defaults) in &defaults.models {
                ensure!(
                    !model.trim().is_empty(),
                    "request parameter model name must not be empty"
                );
                validate_body(kind, &defaults.body).with_context(|| format!("{} model {model}", kind.as_str()))?;
            }
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        kind: RequestKind,
        model: &str,
        base: &Value,
        explicit: &Value,
    ) -> Result<ResolvedRequestParams> {
        self.validate()?;
        ensure!(
            base.is_object() && explicit.is_object(),
            "request base and explicit parameters must be JSON objects"
        );
        let defaults = self.for_kind(kind);
        let model_body = defaults.models.get(model).map(|defaults| &defaults.body);
        let mut body = base.clone();
        let mut layers = vec!["built_in".to_string()];
        deep_merge(&mut body, &defaults.body);
        if defaults.body.as_object().is_some_and(|body| !body.is_empty()) {
            layers.push(kind.as_str().to_string());
        }
        if let Some(model_body) = model_body {
            deep_merge(&mut body, model_body);
            if model_body.as_object().is_some_and(|body| !body.is_empty()) {
                layers.push(format!("{}.models.{model}", kind.as_str()));
            }
        }
        deep_merge(&mut body, explicit);
        if explicit.as_object().is_some_and(|body| !body.is_empty()) {
            layers.push("explicit".to_string());
        }
        let snapshot = json!({"version": self.version, "kind": kind.as_str(), "model": model, "body": defaults.body, "model_body": model_body});
        let digest = Sha256::digest(serde_json::to_vec(&canonical_json(&snapshot))?);
        let config_hash = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(ResolvedRequestParams { body, config_hash, layers })
    }

    pub fn set(&mut self, kind: RequestKind, model: Option<&str>, pointer: &str, value: Value) -> Result<()> {
        let segments = pointer_segments(pointer)?;
        let mut candidate = self.clone();
        let body = candidate.body_mut(kind, model);
        let mut target = body;
        for segment in segments {
            target = target
                .as_object_mut()
                .context("parameter path must descend through JSON objects; set arrays as a whole")?
                .entry(segment)
                .or_insert_with(empty_object);
        }
        deep_merge(target, &value);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn unset(&mut self, kind: RequestKind, model: Option<&str>, pointer: &str) -> Result<()> {
        let mut segments = pointer_segments(pointer)?;
        if segments.is_empty() {
            *self.body_mut(kind, model) = empty_object();
            return Ok(());
        }
        let final_key = segments.pop().unwrap();
        let mut target = self.body_mut(kind, model);
        for segment in segments {
            let object = target
                .as_object_mut()
                .context("parameter path must descend through JSON objects; unset arrays as a whole")?;
            let Some(next) = object.get_mut(&segment) else {
                return Ok(());
            };
            target = next;
        }
        target
            .as_object_mut()
            .context("parameter path must descend through JSON objects; unset arrays as a whole")?
            .remove(&final_key);
        Ok(())
    }

    fn for_kind(&self, kind: RequestKind) -> &TypeDefaults {
        match kind {
            RequestKind::Text => &self.text,
            RequestKind::Image => &self.image,
            RequestKind::Video => &self.video,
        }
    }

    fn body_mut(&mut self, kind: RequestKind, model: Option<&str>) -> &mut Value {
        let defaults = match kind {
            RequestKind::Text => &mut self.text,
            RequestKind::Image => &mut self.image,
            RequestKind::Video => &mut self.video,
        };
        match model {
            Some(model) => &mut defaults.models.entry(model.to_string()).or_default().body,
            None => &mut defaults.body,
        }
    }
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let ordered: BTreeMap<_, _> = object.iter().collect();
            Value::Object(
                ordered
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_json(value)))
                    .collect(),
            )
        }
        Value::Array(array) => Value::Array(array.iter().map(canonical_json).collect()),
        scalar => scalar.clone(),
    }
}

fn validate_body(kind: RequestKind, body: &Value) -> Result<()> {
    let object = body
        .as_object()
        .context("request parameter body must be a JSON object")?;
    ensure!(
        !object.contains_key("model"),
        "configure the model with text-model/image-model/video-model or --model, not request parameters"
    );
    if kind == RequestKind::Text {
        for field in ["messages", "tools", "stream", "stream_options"] {
            ensure!(
                !object.contains_key(field),
                "text request field {field} is managed by the CLI"
            );
        }
    }
    if matches!(kind, RequestKind::Video | RequestKind::Text)
        && let Some(n) = object.get("n")
    {
        ensure!(n.as_u64() == Some(1), "{} n must be 1", kind.as_str());
    }
    Ok(())
}

fn pointer_segments(pointer: &str) -> Result<Vec<String>> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    ensure!(
        pointer.starts_with('/'),
        "parameter path must be a JSON Pointer beginning with /, or empty for the whole body"
    );
    pointer[1..]
        .split('/')
        .map(|part| {
            let mut result = String::new();
            let mut characters = part.chars();
            while let Some(character) = characters.next() {
                if character == '~' {
                    result.push(match characters.next() {
                        Some('0') => '~',
                        Some('1') => '/',
                        _ => bail!("invalid JSON Pointer escape"),
                    });
                } else {
                    result.push(character);
                }
            }
            Ok(result)
        })
        .collect()
}

#[derive(Args)]
pub struct ParamsCmd {
    #[command(subcommand)]
    action: ParamsAction,
}

#[derive(Subcommand)]
enum ParamsAction {
    /// Show the effective configured defaults (without built-in or command values).
    Show {
        kind: RequestKind,
        #[arg(long)]
        model: Option<String>,
    },
    /// Set JSON at a JSON Pointer; objects merge with existing keys.
    Set {
        kind: RequestKind,
        pointer: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
        #[arg(long)]
        model: Option<String>,
    },
    /// Remove a configured field; an empty pointer clears that body.
    Unset {
        kind: RequestKind,
        pointer: String,
        #[arg(long)]
        model: Option<String>,
    },
}

impl ParamsCmd {
    pub fn run(self) -> Result<()> {
        let path = RequestParams::path()?;
        let mut params = RequestParams::load()?;
        match self.action {
            ParamsAction::Show { kind, model } => {
                let cfg = AppConfig::load()?;
                let model = model.unwrap_or(match kind {
                    RequestKind::Text => cfg.text_model,
                    RequestKind::Image => cfg.image_model,
                    RequestKind::Video => cfg.video_model,
                });
                let resolved = params.resolve(kind, &model, &empty_object(), &empty_object())?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"path":path,"kind":kind.as_str(),"model":model,"body":resolved.body,"config_hash":resolved.config_hash,"layers":resolved.layers})
                    )?
                );
            }
            ParamsAction::Set { kind, pointer, value, model } => {
                params.set(
                    kind,
                    model.as_deref(),
                    &pointer,
                    serde_json::from_str(&value)
                        .context("parameter value must be valid JSON; quote strings as JSON strings")?,
                )?;
                params.save_to_path(&path)?;
                println!("updated {} request parameters", kind.as_str());
            }
            ParamsAction::Unset { kind, pointer, model } => {
                params.unset(kind, model.as_deref(), &pointer)?;
                params.save_to_path(&path)?;
                println!("updated {} request parameters", kind.as_str());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("agnes-request-params-{}", rand::random::<u64>()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join(FILE_NAME)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_file_and_empty_configuration_supply_no_extra_parameters() {
        let directory = TempDirectory::new();
        let params = RequestParams::load_from_path(&directory.file()).unwrap();
        assert!(!directory.file().exists());
        for params in [params, serde_json::from_value(json!({})).unwrap()] {
            for kind in [RequestKind::Text, RequestKind::Image, RequestKind::Video] {
                let base = json!({"existing": {"keep": true}});
                let resolved = params.resolve(kind, "fixture", &base, &json!({})).unwrap();
                assert_eq!(resolved.body, base);
                assert_eq!(resolved.layers, ["built_in"]);
            }
        }
    }

    #[test]
    fn explicit_fields_override_defaults_while_unspecified_nested_fields_survive() {
        let params: RequestParams = serde_json::from_value(json!({
            "text": {
                "body": {"temperature": 0.4, "vendor": {"keep": true, "override": 1}, "items": [1, 2]},
                "models": {"fixture": {"body": {"vendor": {"override": 2, "model": "yes"}, "top_p": 0.8}}}
            },
            "image": {"body": {"size": "2K"}},
            "video": {"body": {"seconds": "4"}}
        }))
        .unwrap();
        let base = json!({"temperature": 1, "vendor": {"builtin": 1}, "model": "fixture"});
        let explicit = json!({"temperature": 0, "vendor": {"override": false}, "items": [], "nullable": null});
        let resolved = params.resolve(RequestKind::Text, "fixture", &base, &explicit).unwrap();
        assert_eq!(
            resolved.body,
            json!({
                "temperature": 0,
                "vendor": {"builtin": 1, "keep": true, "override": false, "model": "yes"},
                "items": [], "nullable": null, "model": "fixture", "top_p": 0.8
            })
        );
        assert_eq!(resolved.layers, ["built_in", "text", "text.models.fixture", "explicit"]);
        assert_eq!(
            params
                .resolve(RequestKind::Image, "fixture", &json!({}), &json!({}))
                .unwrap()
                .body,
            json!({"size":"2K"})
        );
        assert_eq!(
            params
                .resolve(RequestKind::Video, "fixture", &json!({}), &json!({}))
                .unwrap()
                .body,
            json!({"seconds":"4"})
        );
        assert_eq!(base["temperature"], 1);
        assert_eq!(explicit["vendor"], json!({"override": false}));
    }

    #[test]
    fn object_set_merges_and_unset_removes_only_the_selected_field() {
        let mut params = RequestParams::default();
        params
            .set(
                RequestKind::Image,
                None,
                "/extra_body",
                json!({"seed": 123, "vendor": {"keep": 1, "change": 1}}),
            )
            .unwrap();
        params
            .set(
                RequestKind::Image,
                None,
                "/extra_body",
                json!({"vendor": {"change": 2}}),
            )
            .unwrap();
        params
            .set(
                RequestKind::Image,
                Some("model.with.dots"),
                "/extra_body/vendor",
                json!({"new": true}),
            )
            .unwrap();
        params
            .unset(RequestKind::Image, None, "/extra_body/vendor/change")
            .unwrap();
        let body = params
            .resolve(RequestKind::Image, "model.with.dots", &json!({}), &json!({}))
            .unwrap()
            .body;
        assert_eq!(body, json!({"extra_body":{"seed":123,"vendor":{"keep":1,"new":true}}}));
        assert_eq!(
            params
                .resolve(RequestKind::Video, "model.with.dots", &json!({}), &json!({}))
                .unwrap()
                .body,
            json!({})
        );
        params.unset(RequestKind::Image, Some("model.with.dots"), "").unwrap();
        assert_eq!(params.image.models["model.with.dots"].body, json!({}));
    }

    #[test]
    fn pointers_handle_literal_slashes_and_tildes_without_array_index_merges() {
        let mut params = RequestParams::default();
        params
            .set(RequestKind::Text, None, "/vendor/a~1b/~0flag", json!([1, 2]))
            .unwrap();
        assert_eq!(params.text.body["vendor"]["a/b"]["~flag"], json!([1, 2]));
        params
            .set(RequestKind::Text, None, "/vendor/a~1b/~0flag", json!([3]))
            .unwrap();
        assert_eq!(params.text.body["vendor"]["a/b"]["~flag"], json!([3]));
        assert!(
            params
                .set(RequestKind::Text, None, "/vendor/a~1b/~0flag/0", json!(4))
                .is_err()
        );
        assert!(params.set(RequestKind::Text, None, "/bad~2escape", json!(4)).is_err());
        assert!(params.unset(RequestKind::Text, None, "/vendor/a~1b/~0flag/0").is_err());
        assert!(params.set(RequestKind::Text, None, "without-slash", json!(4)).is_err());
    }

    #[test]
    fn null_and_unknown_fields_survive_save_reload_and_atomic_replacement() {
        let directory = TempDirectory::new();
        let mut params = RequestParams::default();
        params
            .set(
                RequestKind::Text,
                None,
                "/custom",
                json!({"null":null,"nested":[null,false,3]}),
            )
            .unwrap();
        params.save_to_path(&directory.file()).unwrap();
        params
            .set(RequestKind::Image, None, "/extra_body/vendor", json!(false))
            .unwrap();
        params.save_to_path(&directory.file()).unwrap();
        let loaded = RequestParams::load_from_path(&directory.file()).unwrap();
        assert_eq!(loaded.text.body, params.text.body);
        assert_eq!(loaded.image.body, json!({"extra_body":{"vendor":false}}));
        assert_eq!(
            fs::read_dir(&directory.0).unwrap().count(),
            1,
            "temporary files must not remain"
        );
        let previous = fs::read(directory.file()).unwrap();
        params.video.body = json!({"model":"bad"});
        assert!(params.save_to_path(&directory.file()).is_err());
        assert_eq!(fs::read(directory.file()).unwrap(), previous);
    }

    #[test]
    fn hashes_are_stable_for_key_order_and_do_not_depend_on_runtime_payload() {
        let first: RequestParams = serde_json::from_str(r#"{"text":{"body":{"b":2,"a":1}}}"#).unwrap();
        let second: RequestParams = serde_json::from_str(r#"{"text":{"body":{"a":1,"b":2}}}"#).unwrap();
        let one = first
            .resolve(RequestKind::Text, "m", &json!({"prompt":"one"}), &json!({}))
            .unwrap();
        let two = second
            .resolve(RequestKind::Text, "m", &json!({"prompt":"two"}), &json!({"a":9}))
            .unwrap();
        assert_eq!(one.config_hash, two.config_hash);
        assert_eq!(one.config_hash.len(), 64);
        assert_ne!(
            one.config_hash,
            first
                .resolve(RequestKind::Text, "other", &json!({}), &json!({}))
                .unwrap()
                .config_hash
        );
    }

    #[test]
    fn invalid_schema_and_protocol_overrides_fail_without_mutating_configuration() {
        for invalid in [
            json!({"version":2}),
            json!({"text":{"body":null}}),
            json!({"image":{"body":[]}}),
            json!({"text":{"body":{"stream":false}}}),
            json!({"text":{"body":{"stream_options":{"include_usage":false}}}}),
            json!({"text":{"models":{"m":{"body":{"messages":[]}}}}}),
            json!({"image":{"body":{"model":"other"}}}),
            json!({"video":{"body":{"n":2}}}),
            json!({"text":{"body":{"n":2}}}),
            json!({"video":{"models":{"":{"body":{}}}}}),
        ] {
            let parsed: RequestParams = serde_json::from_value(invalid).unwrap();
            assert!(parsed.validate().is_err());
        }
        assert!(serde_json::from_value::<RequestParams>(json!({"unexpected": {}})).is_err());
        let mut params = RequestParams::default();
        assert!(params.set(RequestKind::Text, None, "/tools", json!([])).is_err());
        assert_eq!(params.text.body, json!({}));
        params.set(RequestKind::Video, None, "/n", json!(1)).unwrap();
        params.validate().unwrap();
    }
}
