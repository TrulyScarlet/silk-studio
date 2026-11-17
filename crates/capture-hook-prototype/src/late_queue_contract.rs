//! Metadata-only policy for authoritative late ResizeBuffers1 observations.
//! IDs must come from retained canonical COM identities, never submission order.
pub const MAX_PRESENT_QUEUES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentQueueIdentity {
    pub queue: u64,
    pub device: u64,
    pub direct: bool,
    pub queue_node_mask: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LateQueueError {
    MissingQueues,
    UnsupportedBufferCount,
    LengthMismatch,
    UnsupportedNodes,
    InvalidQueue,
    WrongDevice,
    AmbiguousQueues,
}

/// Deliberately supports a single-node device and one identical DIRECT queue for
/// every back buffer. Different queues cannot be flattened into a guessed queue.
pub fn validate_present_queues(
    buffer_count: u32,
    device: u64,
    device_nodes: u32,
    creation_masks: &[u32],
    queues: &[PresentQueueIdentity],
) -> Result<PresentQueueIdentity, LateQueueError> {
    if buffer_count < 2 || buffer_count as usize > MAX_PRESENT_QUEUES {
        return Err(LateQueueError::UnsupportedBufferCount);
    }
    if queues.is_empty() {
        return Err(LateQueueError::MissingQueues);
    }
    if queues.len() != buffer_count as usize || creation_masks.len() != queues.len() {
        return Err(LateQueueError::LengthMismatch);
    }
    if device_nodes != 1 || creation_masks.iter().any(|mask| *mask != 1) {
        return Err(LateQueueError::UnsupportedNodes);
    }
    let first = queues[0];
    for queue in queues {
        if queue.queue == 0 || queue.device == 0 || !queue.direct || queue.queue_node_mask > 1 {
            return Err(LateQueueError::InvalidQueue);
        }
        if device == 0 || queue.device != device {
            return Err(LateQueueError::WrongDevice);
        }
        if queue.queue != first.queue {
            return Err(LateQueueError::AmbiguousQueues);
        }
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn direct(queue: u64) -> PresentQueueIdentity {
        PresentQueueIdentity {
            queue,
            device: 7,
            direct: true,
            queue_node_mask: 0,
        }
    }
    #[test]
    fn identical_direct_queue_is_authoritative_for_each_buffer() {
        for count in 2..=MAX_PRESENT_QUEUES {
            assert_eq!(
                validate_present_queues(
                    count as u32,
                    7,
                    1,
                    &vec![1; count],
                    &vec![direct(11); count]
                ),
                Ok(direct(11))
            );
        }
    }
    #[test]
    fn distinct_same_device_queues_are_not_inferred_from_order() {
        for queues in [[direct(11), direct(12)], [direct(12), direct(11)]] {
            assert_eq!(
                validate_present_queues(2, 7, 1, &[1, 1], &queues),
                Err(LateQueueError::AmbiguousQueues)
            );
        }
    }
    #[test]
    fn missing_preserved_oversized_or_mismatched_arrays_do_not_prove_a_queue() {
        assert_eq!(
            validate_present_queues(0, 7, 1, &[], &[]),
            Err(LateQueueError::UnsupportedBufferCount)
        );
        assert_eq!(
            validate_present_queues(17, 7, 1, &[], &[]),
            Err(LateQueueError::UnsupportedBufferCount)
        );
        assert_eq!(
            validate_present_queues(2, 7, 1, &[], &[]),
            Err(LateQueueError::MissingQueues)
        );
        assert_eq!(
            validate_present_queues(2, 7, 1, &[1], &[direct(11); 2]),
            Err(LateQueueError::LengthMismatch)
        );
    }
    #[test]
    fn wrong_device_non_direct_null_and_multi_node_are_rejected() {
        for queue in [
            PresentQueueIdentity {
                device: 9,
                ..direct(11)
            },
            PresentQueueIdentity {
                direct: false,
                ..direct(11)
            },
            direct(0),
            PresentQueueIdentity {
                queue_node_mask: 2,
                ..direct(11)
            },
        ] {
            assert!(validate_present_queues(2, 7, 1, &[1, 1], &[queue; 2]).is_err());
        }
        for masks in [[0, 0], [1, 2], [2, 2]] {
            assert!(validate_present_queues(2, 7, 1, &masks, &[direct(11); 2]).is_err());
        }
        assert_eq!(
            validate_present_queues(2, 7, 2, &[1, 1], &[direct(11); 2]),
            Err(LateQueueError::UnsupportedNodes)
        );
    }
}
