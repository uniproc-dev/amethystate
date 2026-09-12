use amethystate::ReactiveMap;
use amethystate_macros::amethystate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlertThresholds {
    pub warning: u64,
    pub critical: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MonitoringConfig {
    pub enabled: bool,
    pub thresholds: AlertThresholds,
}

#[amethystate]
pub struct DatabaseConfig {
    #[amestate(default = "localhost".to_string())]
    pub host: String,
}

#[amethystate(prefix = "sys")]
pub struct SystemSettings {

    #[amestate(nested)]
    pub db: DatabaseConfig,

    #[amestate(default = MonitoringConfig {
        enabled: true,
        thresholds: AlertThresholds { warning: 50, critical: 80 }
    })]
    pub monitoring: MonitoringConfig,

    #[amestate(default = {
        "cpu": AlertThresholds { warning: 70, critical: 90 },
        "mem": AlertThresholds { warning: 80, critical: 95 }
    })]
    pub limits: ReactiveMap<String, AlertThresholds>,

    #[amestate(default = [
        AlertThresholds { warning: 10, critical: 20 },
        AlertThresholds { warning: 30, critical: 40 }
    ])]
    pub presets: Vec<AlertThresholds>,
}

fn main() {}