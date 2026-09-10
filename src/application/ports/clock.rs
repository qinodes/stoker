use chrono::{DateTime, Utc};

/// Clock used only by application decisions that need an explicit timestamp.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[cfg(test)]
mod tests {
    use super::Clock;
    use chrono::{DateTime, Utc};

    struct FixedClock(DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    #[test]
    fn clock_port_accepts_a_deterministic_test_double() {
        let expected = "2026-01-02T03:04:05Z".parse().unwrap();
        assert_eq!(FixedClock(expected).now(), expected);
    }
}
