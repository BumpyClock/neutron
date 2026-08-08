use collections::HashMap;

#[derive(Debug, Hash, PartialEq, Eq)]
pub(crate) enum SerialKind {
    DataDevice,
    InputMethod,
    MouseEnter,
    MousePress,
    KeyPress,
}

impl SerialKind {
    fn is_input(&self) -> bool {
        matches!(self, SerialKind::MousePress | SerialKind::KeyPress)
    }
}

#[derive(Debug)]
struct SerialData {
    serial: u32,
}

impl SerialData {
    fn new(value: u32) -> Self {
        Self { serial: value }
    }
}

#[derive(Debug)]
/// Helper for tracking of different serial kinds.
pub(crate) struct SerialTracker {
    serials: HashMap<SerialKind, SerialData>,
}

impl SerialTracker {
    pub fn new() -> Self {
        Self {
            serials: HashMap::default(),
        }
    }

    pub fn update(&mut self, kind: SerialKind, value: u32) {
        self.serials.insert(kind, SerialData::new(value));
    }

    /// Returns the latest tracked serial of the provided [`SerialKind`]
    ///
    /// Will return 0 if not tracked.
    pub fn get(&self, kind: SerialKind) -> u32 {
        self.serials
            .get(&kind)
            .map(|serial_data| serial_data.serial)
            .unwrap_or(0)
    }

    /// Returns the most recent input serial.
    ///
    /// Returns 0 only if no input serial has been received yet.
    pub fn get_latest_input(&self) -> u32 {
        latest_serial(
            self.serials
                .iter()
                .filter_map(|(kind, serial_data)| kind.is_input().then_some(serial_data.serial)),
        )
    }
}

fn latest_serial(serials: impl Iterator<Item = u32>) -> u32 {
    serials
        .reduce(|latest, serial| {
            if serial_is_after(serial, latest) {
                serial
            } else {
                latest
            }
        })
        .unwrap_or(0)
}

fn serial_is_after(serial: u32, other: u32) -> bool {
    const SERIAL_WRAP_THRESHOLD: u32 = 1 << 31;

    serial != other && serial.wrapping_sub(other) < SERIAL_WRAP_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_serial_uses_wrapping_order() {
        assert_eq!(latest_serial([u32::MAX - 1, 3].into_iter()), 3);
    }

    #[test]
    fn latest_input_ignores_non_input_serials() {
        let mut tracker = SerialTracker::new();
        tracker.update(SerialKind::KeyPress, 10);
        tracker.update(SerialKind::MouseEnter, 40);
        tracker.update(SerialKind::InputMethod, 50);
        tracker.update(SerialKind::DataDevice, 60);

        assert_eq!(tracker.get_latest_input(), 10);
    }

    #[test]
    fn latest_input_uses_wrapping_order() {
        let mut tracker = SerialTracker::new();
        tracker.update(SerialKind::KeyPress, u32::MAX - 1);
        tracker.update(SerialKind::MousePress, 3);

        assert_eq!(tracker.get_latest_input(), 3);
    }
}
