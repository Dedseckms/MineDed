//! `VoiceSettings` persisted as `<root>/voice.json`.

use std::path::PathBuf;

use lc_runtime::Paths;
use lc_types::voice::{VoiceMode, VoiceSettings};
use lc_types::{AppError, AppResult, ErrorCode};

pub const FILE_NAME: &str = "voice.json";

pub fn defaults() -> VoiceSettings {
    VoiceSettings {
        mode: VoiceMode::Vad,
        input_device: None,
        output_device: None,
        ptt_key: Some("KeyV".to_owned()),
        input_gain: 1.0,
        output_volume: 1.0,
        noise_suppression: true,
        peer_volumes: Default::default(),
    }
}

/// Clamps gains/volumes and drops empty device ids.
pub fn sanitize(mut s: VoiceSettings) -> AppResult<VoiceSettings> {
    let bad = |v: f32| !v.is_finite();
    if bad(s.input_gain) || bad(s.output_volume) || s.peer_volumes.values().any(|v| bad(*v)) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "volume is not a number",
        ));
    }
    s.input_gain = s.input_gain.clamp(0.0, 2.0);
    s.output_volume = s.output_volume.clamp(0.0, 2.0);
    for v in s.peer_volumes.values_mut() {
        *v = v.clamp(0.0, 2.0);
    }
    s.input_device = s.input_device.filter(|d| !d.trim().is_empty());
    s.output_device = s.output_device.filter(|d| !d.trim().is_empty());
    s.ptt_key = s.ptt_key.filter(|k| !k.trim().is_empty());
    Ok(s)
}

#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    pub fn new(paths: &Paths) -> Self {
        Self {
            path: paths.root().join(FILE_NAME),
        }
    }

    /// Missing or corrupt file → defaults (a corrupt file is moved aside).
    pub fn load(&self) -> VoiceSettings {
        match std::fs::read(&self.path) {
            Ok(bytes) => match serde_json::from_slice::<VoiceSettings>(&bytes) {
                Ok(s) => sanitize(s).unwrap_or_else(|_| defaults()),
                Err(_) => {
                    let mut aside = self.path.clone().into_os_string();
                    aside.push(".corrupt");
                    let _ = std::fs::rename(&self.path, aside);
                    defaults()
                }
            },
            Err(_) => defaults(),
        }
    }

    pub async fn save(&self, s: &VoiceSettings) -> AppResult<()> {
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let json = serde_json::to_vec_pretty(s)
            .map_err(|e| AppError::internal(format!("voice settings: {e}")))?;
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_and_corrupt() {
        let dir = std::env::temp_dir().join(format!("lc-voice-settings-{}", std::process::id()));
        let paths = Paths::new(&dir);
        let file = SettingsFile::new(&paths);
        assert_eq!(file.load(), defaults());
        let mut s = defaults();
        s.mode = VoiceMode::PushToTalk;
        s.peer_volumes.insert("uuid-1".into(), 1.5);
        file.save(&s).await.unwrap();
        assert_eq!(SettingsFile::new(&paths).load(), s);
        std::fs::write(dir.join(FILE_NAME), b"{oops").unwrap();
        assert_eq!(file.load(), defaults());
        assert!(dir.join("voice.json.corrupt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_clamps() {
        let mut s = defaults();
        s.input_gain = 5.0;
        s.output_volume = -1.0;
        s.input_device = Some(" ".into());
        let s = sanitize(s).unwrap();
        assert_eq!((s.input_gain, s.output_volume), (2.0, 0.0));
        assert!(s.input_device.is_none());
        let mut bad = defaults();
        bad.input_gain = f32::NAN;
        assert!(sanitize(bad).is_err());
    }
}
