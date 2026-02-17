use crate::frame;

#[derive(Debug, Clone)]
pub struct Config {
    pub max_sessions: u64,
    pub initial_max_streams_uni: u64,
    pub initial_max_streams_bidi: u64,
    pub initial_max_data: u64,
    pub max_buffered_streams: usize,
    pub max_buffered_datagrams: usize,
    pub enable_datagrams: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_sessions: 1,
            initial_max_streams_uni: 100,
            initial_max_streams_bidi: 100,
            initial_max_data: 1_048_576,
            max_buffered_streams: 16,
            max_buffered_datagrams: 16,
            enable_datagrams: true,
        }
    }
}

impl Config {
    /// Produce the list of SETTINGS key-value pairs that must be sent in the
    /// H3 SETTINGS frame to advertise WebTransport capability.
    pub fn h3_settings(&self) -> Vec<(u64, u64)> {
        let mut s = vec![
            (frame::SETTINGS_WT_MAX_SESSIONS, self.max_sessions),
            (frame::SETTINGS_ENABLE_CONNECT_PROTOCOL, 1),
            (frame::SETTINGS_QPACK_MAX_TABLE_CAPACITY, 0),
            (frame::SETTINGS_QPACK_BLOCKED_STREAMS, 0),
        ];

        if self.enable_datagrams {
            s.push((frame::SETTINGS_H3_DATAGRAM, 1));
        }

        if self.initial_max_streams_uni > 0 {
            s.push((
                frame::SETTINGS_WT_INITIAL_MAX_STREAMS_UNI,
                self.initial_max_streams_uni,
            ));
        }
        if self.initial_max_streams_bidi > 0 {
            s.push((
                frame::SETTINGS_WT_INITIAL_MAX_STREAMS_BIDI,
                self.initial_max_streams_bidi,
            ));
        }
        if self.initial_max_data > 0 {
            s.push((frame::SETTINGS_WT_INITIAL_MAX_DATA, self.initial_max_data));
        }

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_include_required() {
        let cfg = Config::default();
        let settings = cfg.h3_settings();

        let has = |id: u64| settings.iter().any(|(k, _)| *k == id);
        assert!(has(frame::SETTINGS_WT_MAX_SESSIONS));
        assert!(has(frame::SETTINGS_ENABLE_CONNECT_PROTOCOL));
        assert!(has(frame::SETTINGS_H3_DATAGRAM));
    }

    #[test]
    fn datagrams_disabled_omits_setting() {
        let cfg = Config {
            enable_datagrams: false,
            ..Config::default()
        };
        let settings = cfg.h3_settings();
        let has_dgram = settings
            .iter()
            .any(|(k, _)| *k == frame::SETTINGS_H3_DATAGRAM);
        assert!(!has_dgram);
    }
}
