//! Keep transport failures distinct from an authenticated service exit report.
//!
//! `None` means unknown, not stopped. Startup may fail early only on
//! `Some(false)`; unknown probes remain subject to the startup deadline.
pub fn from_response<E>(response: Result<(bool, Option<bool>), E>) -> Option<bool> {
    match response {
        Ok((true, running)) => running,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_failure_is_unknown_not_stopped() {
        let status = from_response::<&str>(Err("service exchange deadline exceeded"));
        assert_eq!(status, None);
        assert_ne!(status, Some(false));
    }

    #[test]
    fn transient_failure_can_be_followed_by_a_healthy_core() {
        let replies = [Err("temporary IPC failure"), Ok((true, Some(true)))];
        let statuses: Vec<_> = replies.into_iter().map(from_response).collect();
        assert_eq!(statuses, [None, Some(true)]);
        assert!(!statuses.contains(&Some(false)));
    }

    #[test]
    fn only_a_successful_explicit_stopped_report_confirms_exit() {
        assert_eq!(from_response::<()>(Ok((true, Some(false)))), Some(false));
        assert_eq!(from_response::<()>(Ok((true, Some(true)))), Some(true));
    }

    #[test]
    fn failed_or_incomplete_response_is_unknown() {
        assert_eq!(from_response::<()>(Ok((false, Some(false)))), None);
        assert_eq!(from_response::<()>(Ok((false, Some(true)))), None);
        assert_eq!(from_response::<()>(Ok((true, None))), None);
    }
}
