use crate::capsule::Capsule;
use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct FlowController {
    local_max_streams_bidi: u64,
    local_max_streams_uni: u64,
    peer_max_streams_bidi: u64,
    peer_max_streams_uni: u64,
    streams_opened_bidi: u64,
    streams_opened_uni: u64,
    peer_streams_opened_bidi: u64,
    peer_streams_opened_uni: u64,

    local_max_data: u64,
    peer_max_data: u64,
    data_sent: u64,
    data_received: u64,

    enabled: bool,
}

impl FlowController {
    pub fn new(
        initial_max_streams_bidi: u64,
        initial_max_streams_uni: u64,
        initial_max_data: u64,
    ) -> Self {
        let enabled =
            initial_max_streams_bidi > 0 || initial_max_streams_uni > 0 || initial_max_data > 0;

        Self {
            local_max_streams_bidi: initial_max_streams_bidi,
            local_max_streams_uni: initial_max_streams_uni,
            peer_max_streams_bidi: 0,
            peer_max_streams_uni: 0,
            streams_opened_bidi: 0,
            streams_opened_uni: 0,
            peer_streams_opened_bidi: 0,
            peer_streams_opened_uni: 0,
            local_max_data: initial_max_data,
            peer_max_data: 0,
            data_sent: 0,
            data_received: 0,
            enabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn can_open_bidi(&self) -> bool {
        !self.enabled || self.streams_opened_bidi < self.peer_max_streams_bidi
    }

    pub fn can_open_uni(&self) -> bool {
        !self.enabled || self.streams_opened_uni < self.peer_max_streams_uni
    }

    pub fn can_send(&self, len: u64) -> bool {
        !self.enabled || self.data_sent + len <= self.peer_max_data
    }

    pub fn on_stream_opened_local(&mut self, bidi: bool) {
        if bidi {
            self.streams_opened_bidi += 1;
        } else {
            self.streams_opened_uni += 1;
        }
    }

    pub fn on_stream_opened_remote(&mut self, bidi: bool) {
        if bidi {
            self.peer_streams_opened_bidi += 1;
        } else {
            self.peer_streams_opened_uni += 1;
        }
    }

    pub fn on_data_sent(&mut self, len: u64) {
        self.data_sent += len;
    }

    pub fn on_data_received(&mut self, len: u64) {
        self.data_received += len;
    }

    // §5.6.2: WT_MAX_STREAMS — values must be monotonically increasing
    pub fn apply_peer_max_streams_bidi(&mut self, max: u64) -> Result<()> {
        if self.enabled && max < self.peer_max_streams_bidi {
            return Err(Error::FlowControlError);
        }
        self.peer_max_streams_bidi = max;
        Ok(())
    }

    pub fn apply_peer_max_streams_uni(&mut self, max: u64) -> Result<()> {
        if self.enabled && max < self.peer_max_streams_uni {
            return Err(Error::FlowControlError);
        }
        self.peer_max_streams_uni = max;
        Ok(())
    }

    // §5.6.4: WT_MAX_DATA — values must be monotonically increasing
    pub fn apply_peer_max_data(&mut self, max: u64) -> Result<()> {
        if self.enabled && max < self.peer_max_data {
            return Err(Error::FlowControlError);
        }
        self.peer_max_data = max;
        Ok(())
    }

    pub fn set_peer_initial_limits(
        &mut self,
        max_streams_bidi: u64,
        max_streams_uni: u64,
        max_data: u64,
    ) {
        self.peer_max_streams_bidi = max_streams_bidi;
        self.peer_max_streams_uni = max_streams_uni;
        self.peer_max_data = max_data;
    }

    pub fn local_max_streams_bidi(&self) -> u64 {
        self.local_max_streams_bidi
    }

    pub fn local_max_streams_uni(&self) -> u64 {
        self.local_max_streams_uni
    }

    pub fn local_max_data(&self) -> u64 {
        self.local_max_data
    }

    pub fn peer_max_streams_bidi(&self) -> u64 {
        self.peer_max_streams_bidi
    }

    pub fn peer_max_streams_uni(&self) -> u64 {
        self.peer_max_streams_uni
    }

    pub fn peer_max_data(&self) -> u64 {
        self.peer_max_data
    }

    pub fn process_capsule(&mut self, capsule: &Capsule) -> Result<()> {
        match capsule {
            Capsule::MaxStreamsBidi(v) => self.apply_peer_max_streams_bidi(*v),
            Capsule::MaxStreamsUni(v) => self.apply_peer_max_streams_uni(*v),
            Capsule::MaxData(v) => self.apply_peer_max_data(*v),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_allows_nothing_when_enabled() {
        let fc = FlowController::new(10, 10, 1000);
        assert!(!fc.can_open_bidi());
        assert!(!fc.can_open_uni());
        assert!(!fc.can_send(1));
    }

    #[test]
    fn peer_limits_unlock_operations() {
        let mut fc = FlowController::new(10, 10, 1000);
        fc.apply_peer_max_streams_bidi(5).unwrap();
        fc.apply_peer_max_streams_uni(5).unwrap();
        fc.apply_peer_max_data(500).unwrap();

        assert!(fc.can_open_bidi());
        assert!(fc.can_open_uni());
        assert!(fc.can_send(500));
        assert!(!fc.can_send(501));
    }

    #[test]
    fn stream_limit_enforced() {
        let mut fc = FlowController::new(10, 10, 1000);
        fc.apply_peer_max_streams_bidi(2).unwrap();

        assert!(fc.can_open_bidi());
        fc.on_stream_opened_local(true);
        assert!(fc.can_open_bidi());
        fc.on_stream_opened_local(true);
        assert!(!fc.can_open_bidi());
    }

    #[test]
    fn decreasing_max_streams_rejected() {
        let mut fc = FlowController::new(10, 10, 1000);
        fc.apply_peer_max_streams_bidi(10).unwrap();
        assert!(fc.apply_peer_max_streams_bidi(5).is_err());
    }

    #[test]
    fn decreasing_max_data_rejected() {
        let mut fc = FlowController::new(10, 10, 1000);
        fc.apply_peer_max_data(1000).unwrap();
        assert!(fc.apply_peer_max_data(500).is_err());
    }

    #[test]
    fn disabled_flow_control_allows_everything() {
        let fc = FlowController::new(0, 0, 0);
        assert!(!fc.is_enabled());
        assert!(fc.can_open_bidi());
        assert!(fc.can_open_uni());
        assert!(fc.can_send(u64::MAX));
    }
}
