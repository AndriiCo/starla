//! Configuration management

use serde::{Deserialize, Serialize};

/// Main probe configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProbeConfig {
    #[serde(default)]
    pub probe: ProbeSettings,

    #[serde(default)]
    pub network: NetworkSettings,

    #[serde(default)]
    pub controller: ControllerSettings,

    #[serde(default)]
    pub storage: StorageSettings,

    #[serde(default)]
    pub measurements: MeasurementSettings,

    #[serde(default)]
    pub metrics: MetricsSettings,

    #[serde(default)]
    pub logging: LoggingSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeSettings {
    /// Log level
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Default for ProbeSettings {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkSettings {
    #[serde(default)]
    pub rxtxrpt: bool,

    /// Enable the status socket for tray app communication.
    /// Only needed on desktop systems. Default: false.
    #[serde(default)]
    pub status_socket: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerSettings {
    #[serde(default = "default_registration_servers")]
    pub registration_servers: Vec<String>,

    #[serde(default = "default_ssh_timeout")]
    pub ssh_timeout: u64,

    #[serde(default = "default_keepalive_interval")]
    pub keepalive_interval: u64,
}

impl Default for ControllerSettings {
    fn default() -> Self {
        Self {
            registration_servers: default_registration_servers(),
            ssh_timeout: default_ssh_timeout(),
            keepalive_interval: default_keepalive_interval(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSettings {
    /// Maximum number of results to hold in the upload queue
    #[serde(default = "default_max_queue_size")]
    pub max_queue_size: usize,

    /// No longer used (Starla keeps no results to expire): accepted so that
    /// a config that sets it still loads, and reported by
    /// [`ProbeConfig::ignored_keys`].
    #[serde(default, skip_serializing)]
    pub retention_days: Option<toml::Value>,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            max_queue_size: default_max_queue_size(),
            retention_days: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MeasurementSettings {
    /// Time replies by the kernel's receive stamps (SO_TIMESTAMPNS), not
    /// by a userspace clock read once they are read, as the reference
    /// probe does. Linux only; off by default. See
    /// starla_network::rxstamp.
    #[serde(default)]
    pub kernel_rx_timestamps: bool,
    /// Run the background threads on SCHED_IDLE as well as 10 nice levels
    /// lower, so that a measurement thread woken on their CPU preempts
    /// them at once. Linux only; off by default. See
    /// starla_common::priority::BACKGROUND_NICE_INCREMENT.
    #[serde(default)]
    pub background_sched_idle: bool,
    /// Hold the NTP runs to a server after its kiss-o'-death: a RATE for a
    /// minute and up, doubling, a DENY or RSTR for a day, as RFC 5905 §7.4
    /// asks; a held run sends nothing and reports `not sent: KoD …`. Off by
    /// default: the reference probe runs every line on its slot whatever
    /// the server said. See starla_measurements::ntp::kod.
    #[serde(default)]
    pub honor_ntp_kod: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSettings {
    #[serde(default = "default_metrics_enabled")]
    pub enabled: bool,

    #[serde(default = "default_metrics_listen_addr")]
    pub listen_addr: String,
}

impl Default for MetricsSettings {
    fn default() -> Self {
        Self {
            enabled: default_metrics_enabled(),
            listen_addr: default_metrics_listen_addr(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingSettings {
    #[serde(default = "default_log_format")]
    pub format: String,

    /// With `--log-dir`, how many daily log files to keep: at each start
    /// and each rotation the oldest `probe.log.*` files beyond this many
    /// are deleted. 0 keeps them all.
    #[serde(default = "default_max_files")]
    pub max_files: u32,

    /// No longer used: the log goes to stdout, or to daily files in
    /// `--log-dir` when that is given. Accepted so that a config that sets
    /// it still loads, and reported by [`ProbeConfig::ignored_keys`].
    #[serde(default, skip_serializing)]
    pub output: Option<toml::Value>,

    /// No longer used: log files rotate daily, not by size. Accepted so
    /// that a config that sets it still loads, and reported by
    /// [`ProbeConfig::ignored_keys`].
    #[serde(default, skip_serializing)]
    pub max_file_size_mb: Option<toml::Value>,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            format: default_log_format(),
            max_files: default_max_files(),
            output: None,
            max_file_size_mb: None,
        }
    }
}

// Default value functions
fn default_log_level() -> String {
    "info".to_string()
}
fn default_registration_servers() -> Vec<String> {
    vec![
        // RIPE's production registration servers since release 5130
        // (config/probe/reg_servers.sh.prod, 2026-07-13), which replaced
        // reg03/reg04. Hostname, IPv4, IPv6 for each, in that order.
        "reg-05.prod.atlas.ripe.net:443".to_string(),
        "193.0.19.75:443".to_string(),
        "[2001:67c:2e8:11::c100:134b]:443".to_string(),
        "reg-06.prod.atlas.ripe.net:443".to_string(),
        "193.0.19.76:443".to_string(),
        "[2001:67c:2e8:11::c100:134c]:443".to_string(),
    ]
}
fn default_ssh_timeout() -> u64 {
    30
}
fn default_keepalive_interval() -> u64 {
    60
}
fn default_max_queue_size() -> usize {
    10000
}
fn default_metrics_enabled() -> bool {
    true
}
fn default_metrics_listen_addr() -> String {
    "127.0.0.1:9695".to_string()
}
fn default_log_format() -> String {
    "json".to_string()
}
fn default_max_files() -> u32 {
    5
}

impl ProbeConfig {
    /// Load configuration from TOML file
    pub fn from_file(path: &std::path::Path) -> Result<Self, crate::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::Error::Config(format!("Failed to read config: {}", e)))?;

        let config: ProbeConfig = toml::from_str(&content)
            .map_err(|e| crate::Error::Config(format!("Failed to parse config: {}", e)))?;

        Ok(config)
    }

    /// The keys this config sets that Starla no longer reads, each with
    /// what to do instead, for a warning at startup.
    pub fn ignored_keys(&self) -> Vec<(&'static str, &'static str)> {
        let mut keys = Vec::new();
        if self.logging.output.is_some() {
            keys.push((
                "logging.output",
                "the log goes to stdout, or to daily files with --log-dir",
            ));
        }
        if self.logging.max_file_size_mb.is_some() {
            keys.push((
                "logging.max_file_size_mb",
                "log files rotate daily; logging.max_files caps how many are kept",
            ));
        }
        if self.storage.retention_days.is_some() {
            keys.push((
                "storage.retention_days",
                "Starla keeps no results on disk to expire",
            ));
        }
        keys
    }

    /// Save configuration to TOML file
    pub fn to_file(&self, path: &std::path::Path) -> Result<(), crate::Error> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| crate::Error::Config(format!("Failed to serialize config: {}", e)))?;

        std::fs::write(path, content)
            .map_err(|e| crate::Error::Config(format!("Failed to write config: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ProbeConfig::default();
        assert_eq!(config.probe.log_level, "info");
        assert!(!config.network.rxtxrpt);
    }

    /// Off unless asked for: by default, and in a config without the
    /// section or the key.
    #[test]
    fn background_sched_idle_is_off_by_default() {
        assert!(!ProbeConfig::default().measurements.background_sched_idle);
        let parsed: ProbeConfig = toml::from_str("[measurements]\n").unwrap();
        assert!(!parsed.measurements.background_sched_idle);
        let on = "[measurements]\nbackground_sched_idle = true\n";
        let parsed: ProbeConfig = toml::from_str(on).unwrap();
        assert!(parsed.measurements.background_sched_idle);
    }

    /// Off unless asked for: by default, and in a config without the
    /// section or the key.
    #[test]
    fn honor_ntp_kod_is_off_by_default() {
        assert!(!ProbeConfig::default().measurements.honor_ntp_kod);
        let parsed: ProbeConfig = toml::from_str("[measurements]\n").unwrap();
        assert!(!parsed.measurements.honor_ntp_kod);
        let on = "[measurements]\nhonor_ntp_kod = true\n";
        let parsed: ProbeConfig = toml::from_str(on).unwrap();
        assert!(parsed.measurements.honor_ntp_kod);
    }

    #[test]
    fn kernel_rx_timestamps_are_off_by_default() {
        assert!(!ProbeConfig::default().measurements.kernel_rx_timestamps);
        let parsed: ProbeConfig = toml::from_str("[probe]\nlog_level = \"debug\"\n").unwrap();
        assert!(!parsed.measurements.kernel_rx_timestamps);
        let parsed: ProbeConfig = toml::from_str("[measurements]\n").unwrap();
        assert!(!parsed.measurements.kernel_rx_timestamps);
        let on = "[measurements]\nkernel_rx_timestamps = true\n";
        let parsed: ProbeConfig = toml::from_str(on).unwrap();
        assert!(parsed.measurements.kernel_rx_timestamps);
    }

    /// The example config parses, and leaves the option off.
    #[test]
    fn the_example_config_parses() {
        let example = include_str!("../../../config.toml.example");
        let parsed: ProbeConfig = toml::from_str(example).unwrap();
        assert!(!parsed.measurements.kernel_rx_timestamps);
        assert!(example.contains("\nkernel_rx_timestamps = false\n"));
        assert!(!parsed.measurements.background_sched_idle);
        assert!(example.contains("\nbackground_sched_idle = false\n"));
        assert!(!parsed.measurements.honor_ntp_kod);
        assert!(example.contains("\nhonor_ntp_kod = false\n"));
        assert!(parsed.ignored_keys().is_empty());
    }

    /// A config written for an earlier Starla, with the keys it no longer
    /// reads, still loads; each of those keys is reported, and the rest
    /// of the config is read as before.
    #[test]
    fn a_config_with_keys_no_longer_read_loads_and_names_them() {
        let old = "[storage]\nmax_queue_size = 500\nretention_days = 30\n[logging]\nformat = \
                   \"text\"\noutput = \"file\"\nmax_file_size_mb = 10\nmax_files = 3\n";
        let parsed: ProbeConfig = toml::from_str(old).unwrap();
        assert_eq!(parsed.storage.max_queue_size, 500);
        assert_eq!(parsed.logging.format, "text");
        assert_eq!(parsed.logging.max_files, 3);
        let ignored: Vec<_> = parsed.ignored_keys().into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            ignored,
            [
                "logging.output",
                "logging.max_file_size_mb",
                "storage.retention_days"
            ]
        );

        // Nothing to report for a config without them, or for the
        // defaults; and they are not written back out.
        let parsed: ProbeConfig = toml::from_str("[logging]\nformat = \"text\"\n").unwrap();
        assert!(parsed.ignored_keys().is_empty());
        assert_eq!(parsed.logging.max_files, 5);
        assert!(ProbeConfig::default().ignored_keys().is_empty());
        let written = toml::to_string(&ProbeConfig::default()).unwrap();
        assert!(!written.contains("retention_days"), "{written}");
        assert!(!written.contains("output"), "{written}");
        assert!(!written.contains("max_file_size_mb"), "{written}");
    }

    #[test]
    fn test_config_serde() {
        let config = ProbeConfig::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: ProbeConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.probe.log_level, "info");
    }
}
