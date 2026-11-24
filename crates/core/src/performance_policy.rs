//! Deterministic policy evaluation for automatic playback and rendering pauses.

use serde::{Deserialize, Deserializer, Serialize};

pub const PERFORMANCE_POLICY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    #[default]
    Continue,
    PauseRendering,
    PausePlayback,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformancePolicySettings {
    schema_version: u32,
    fullscreen_action: PolicyAction,
    session_lock_action: PolicyAction,
    battery_action: PolicyAction,
    #[serde(default)]
    battery_fps_limit: Option<u16>,
    #[serde(default = "default_sharp_upscaling")]
    sharp_upscaling: bool,
}

const fn default_sharp_upscaling() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerformancePolicyValidationError {
    BatteryFpsOutOfRange,
    BatteryFpsConflictsWithPause,
}

impl<'de> Deserialize<'de> for PerformancePolicySettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct WireSettings {
            schema_version: u32,
            fullscreen_action: PolicyAction,
            session_lock_action: PolicyAction,
            battery_action: PolicyAction,
            #[serde(default)]
            battery_fps_limit: Option<u16>,
            #[serde(default = "default_sharp_upscaling")]
            sharp_upscaling: bool,
        }

        let value = WireSettings::deserialize(deserializer)?;
        if value.schema_version != PERFORMANCE_POLICY_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(
                "unsupported performance policy schema",
            ));
        }
        if let Some(limit) = value.battery_fps_limit {
            if !(1..=120).contains(&limit) {
                return Err(serde::de::Error::custom(
                    "battery FPS limit is out of range",
                ));
            }
            if value.battery_action != PolicyAction::Continue {
                return Err(serde::de::Error::custom(
                    "battery FPS limit conflicts with the battery pause action",
                ));
            }
        }
        Ok(Self {
            schema_version: value.schema_version,
            fullscreen_action: value.fullscreen_action,
            session_lock_action: value.session_lock_action,
            battery_action: value.battery_action,
            battery_fps_limit: value.battery_fps_limit,
            sharp_upscaling: value.sharp_upscaling,
        })
    }
}

impl Default for PerformancePolicySettings {
    fn default() -> Self {
        Self {
            schema_version: PERFORMANCE_POLICY_SCHEMA_VERSION,
            fullscreen_action: PolicyAction::PausePlayback,
            session_lock_action: PolicyAction::PausePlayback,
            battery_action: PolicyAction::Continue,
            battery_fps_limit: None,
            sharp_upscaling: true,
        }
    }
}

impl PerformancePolicySettings {
    pub const fn new(
        fullscreen_action: PolicyAction,
        session_lock_action: PolicyAction,
        battery_action: PolicyAction,
    ) -> Self {
        Self {
            schema_version: PERFORMANCE_POLICY_SCHEMA_VERSION,
            fullscreen_action,
            session_lock_action,
            battery_action,
            battery_fps_limit: None,
            sharp_upscaling: true,
        }
    }

    pub const fn sharp_upscaling(&self) -> bool {
        self.sharp_upscaling
    }

    pub const fn with_sharp_upscaling(mut self, enabled: bool) -> Self {
        self.sharp_upscaling = enabled;
        self
    }

    pub const fn fullscreen_action(&self) -> PolicyAction {
        self.fullscreen_action
    }

    pub const fn session_lock_action(&self) -> PolicyAction {
        self.session_lock_action
    }

    pub const fn battery_action(&self) -> PolicyAction {
        self.battery_action
    }

    pub const fn battery_fps_limit(&self) -> Option<u16> {
        self.battery_fps_limit
    }

    pub fn with_battery_fps_limit(
        mut self,
        limit: Option<u16>,
    ) -> Result<Self, PerformancePolicyValidationError> {
        if let Some(limit) = limit {
            if !(1..=120).contains(&limit) {
                return Err(PerformancePolicyValidationError::BatteryFpsOutOfRange);
            }
            if self.battery_action != PolicyAction::Continue {
                return Err(PerformancePolicyValidationError::BatteryFpsConflictsWithPause);
            }
        }
        self.battery_fps_limit = limit;
        Ok(self)
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub const fn evaluate(
        &self,
        signals: PerformanceSignals,
        user_paused: bool,
    ) -> PerformanceDecision {
        let mut action = if user_paused {
            PolicyAction::PausePlayback
        } else {
            PolicyAction::Continue
        };

        if signals.fullscreen {
            action = stronger(action, self.fullscreen_action);
        }
        if signals.session_locked {
            action = stronger(action, self.session_lock_action);
        }
        if signals.on_battery {
            action = stronger(action, self.battery_action);
        }

        PerformanceDecision {
            action,
            active_signals: signals,
            user_paused,
            battery_fps_limit: if signals.on_battery {
                self.battery_fps_limit
            } else {
                None
            },
        }
    }
}

const fn stronger(left: PolicyAction, right: PolicyAction) -> PolicyAction {
    if action_rank(right) > action_rank(left) {
        right
    } else {
        left
    }
}

const fn action_rank(action: PolicyAction) -> u8 {
    match action {
        PolicyAction::Continue => 0,
        PolicyAction::PauseRendering => 1,
        PolicyAction::PausePlayback => 2,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerformanceSignals {
    pub fullscreen: bool,
    pub session_locked: bool,
    pub on_battery: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerformanceDecision {
    action: PolicyAction,
    active_signals: PerformanceSignals,
    user_paused: bool,
    battery_fps_limit: Option<u16>,
}

impl PerformanceDecision {
    pub const fn action(self) -> PolicyAction {
        self.action
    }

    pub const fn active_signals(self) -> PerformanceSignals {
        self.active_signals
    }

    pub const fn user_paused(self) -> bool {
        self.user_paused
    }

    pub const fn battery_fps_limit(self) -> Option<u16> {
        self.battery_fps_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharp_upscaling_is_on_by_default_and_for_files_without_the_field() {
        assert!(PerformancePolicySettings::default().sharp_upscaling());
        let old = r#"{"schemaVersion":1,"fullscreenAction":"continue","sessionLockAction":"continue","batteryAction":"continue"}"#;
        let loaded: PerformancePolicySettings = serde_json::from_str(old).unwrap();
        assert!(loaded.sharp_upscaling());
    }

    #[test]
    fn sharp_upscaling_can_be_turned_off_and_round_trips() {
        let off = PerformancePolicySettings::default().with_sharp_upscaling(false);
        let json = serde_json::to_string(&off).unwrap();
        let loaded: PerformancePolicySettings = serde_json::from_str(&json).unwrap();
        assert!(!loaded.sharp_upscaling());
    }

    #[test]
    fn defaults_pause_on_fullscreen_and_session_lock() {
        let settings = PerformancePolicySettings::default();
        assert_eq!(
            settings
                .evaluate(
                    PerformanceSignals {
                        fullscreen: true,
                        ..PerformanceSignals::default()
                    },
                    false
                )
                .action(),
            PolicyAction::PausePlayback
        );
        assert_eq!(
            settings
                .evaluate(
                    PerformanceSignals {
                        session_locked: true,
                        ..PerformanceSignals::default()
                    },
                    false
                )
                .action(),
            PolicyAction::PausePlayback
        );
    }

    #[test]
    fn strongest_action_wins_independent_of_signal_order() {
        let settings = PerformancePolicySettings::new(
            PolicyAction::PauseRendering,
            PolicyAction::PausePlayback,
            PolicyAction::Continue,
        );
        let decision = settings.evaluate(
            PerformanceSignals {
                fullscreen: true,
                session_locked: true,
                on_battery: true,
            },
            false,
        );
        assert_eq!(decision.action(), PolicyAction::PausePlayback);
        assert!(decision.active_signals().on_battery);
    }

    #[test]
    fn clearing_one_signal_preserves_other_active_policy() {
        let settings = PerformancePolicySettings::new(
            PolicyAction::PausePlayback,
            PolicyAction::PauseRendering,
            PolicyAction::Continue,
        );
        assert_eq!(
            settings
                .evaluate(
                    PerformanceSignals {
                        fullscreen: true,
                        session_locked: true,
                        on_battery: false,
                    },
                    false
                )
                .action(),
            PolicyAction::PausePlayback
        );
        assert_eq!(
            settings
                .evaluate(
                    PerformanceSignals {
                        fullscreen: false,
                        session_locked: true,
                        on_battery: false,
                    },
                    false
                )
                .action(),
            PolicyAction::PauseRendering
        );
    }

    #[test]
    fn manual_pause_remains_after_automatic_signals_clear() {
        let settings = PerformancePolicySettings::new(
            PolicyAction::Continue,
            PolicyAction::Continue,
            PolicyAction::Continue,
        );
        let decision = settings.evaluate(PerformanceSignals::default(), true);
        assert_eq!(decision.action(), PolicyAction::PausePlayback);
        assert!(decision.user_paused());
    }

    #[test]
    fn deserialization_rejects_unknown_fields_and_schema_versions() {
        let unknown_field = r#"{"schemaVersion":1,"fullscreenAction":"continue","sessionLockAction":"continue","batteryAction":"continue","extra":true}"#;
        let unsupported = r#"{"schemaVersion":2,"fullscreenAction":"continue","sessionLockAction":"continue","batteryAction":"continue"}"#;
        assert!(serde_json::from_str::<PerformancePolicySettings>(unknown_field).is_err());
        assert!(serde_json::from_str::<PerformancePolicySettings>(unsupported).is_err());
    }

    #[test]
    fn battery_fps_limit_is_bounded_and_only_active_on_battery() {
        let settings = PerformancePolicySettings::default()
            .with_battery_fps_limit(Some(30))
            .unwrap();
        assert_eq!(
            settings
                .evaluate(
                    PerformanceSignals {
                        on_battery: true,
                        ..PerformanceSignals::default()
                    },
                    false,
                )
                .battery_fps_limit(),
            Some(30)
        );
        assert_eq!(
            settings
                .evaluate(PerformanceSignals::default(), false)
                .battery_fps_limit(),
            None
        );
        assert!(settings.clone().with_battery_fps_limit(Some(0)).is_err());
        assert!(settings.with_battery_fps_limit(Some(121)).is_err());
        assert!(
            PerformancePolicySettings::new(
                PolicyAction::Continue,
                PolicyAction::Continue,
                PolicyAction::PausePlayback,
            )
            .with_battery_fps_limit(Some(30))
            .is_err()
        );
    }

    #[test]
    fn old_schema_one_policy_defaults_missing_battery_cap_to_none() {
        let old = r#"{"schemaVersion":1,"fullscreenAction":"pause_playback","sessionLockAction":"pause_playback","batteryAction":"continue"}"#;
        let settings: PerformancePolicySettings = serde_json::from_str(old).unwrap();
        assert_eq!(settings.battery_fps_limit(), None);
    }
}
