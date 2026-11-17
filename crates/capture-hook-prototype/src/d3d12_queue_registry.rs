//! Bounded metadata contract for future owned CreateSwapChainForHwnd observation.
//! IDs are opaque canonical identities, never pointers this module dereferences.
//! This does not discover queues, retain COM objects, or authorize attachment.

/// Fixed maximum retained associations, with no callback-time allocation.
pub const ASSOCIATION_CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Association {
    pub chain: u64,
    pub queue: u64,
    pub device: u64,
    pub window: u64,
}

/// Registry-local token, not an IPC credential or a cross-registry identity.
/// Generation prevents a retired identity/address from satisfying an old selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssociationToken {
    association: Association,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    Closed,
    InvalidIdentity,
    Conflict,
    Full,
    GenerationExhausted,
    Stale,
    AlreadySelected,
    NotSelected,
}

pub struct QueueRegistry {
    entries: [Option<AssociationToken>; ASSOCIATION_CAPACITY],
    next_generation: u64,
    selected: Option<AssociationToken>,
    closed: bool,
}

impl Default for QueueRegistry {
    fn default() -> Self {
        Self {
            entries: [None; ASSOCIATION_CAPACITY],
            next_generation: 1,
            selected: None,
            closed: false,
        }
    }
}

impl QueueRegistry {
    /// Call only after successful creation and native DIRECT/device validation.
    /// Exact repeated observations are idempotent; conflicts never replace entries.
    pub fn observe(&mut self, association: Association) -> Result<AssociationToken, RegistryError> {
        self.ensure_open()?;
        if [
            association.chain,
            association.queue,
            association.device,
            association.window,
        ]
        .contains(&0)
        {
            return Err(RegistryError::InvalidIdentity);
        }
        for entry in self.entries.iter().flatten() {
            let old = entry.association;
            if old == association {
                return Ok(*entry);
            }
            if old.chain == association.chain
                || old.window == association.window
                || (old.queue == association.queue && old.device != association.device)
            {
                return Err(RegistryError::Conflict);
            }
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(RegistryError::Full)?;
        let next = self
            .next_generation
            .checked_add(1)
            .ok_or(RegistryError::GenerationExhausted)?;
        let token = AssociationToken {
            association,
            generation: self.next_generation,
        };
        *slot = Some(token);
        self.next_generation = next;
        Ok(token)
    }

    /// Explicit token selection, never first-seen/foreground/last-submitted queue.
    pub fn select(&mut self, token: AssociationToken) -> Result<(), RegistryError> {
        self.ensure_open()?;
        if !self.entries.contains(&Some(token)) {
            return Err(RegistryError::Stale);
        }
        if let Some(selected) = self.selected {
            return if selected == token {
                Ok(())
            } else {
                Err(RegistryError::AlreadySelected)
            };
        }
        self.selected = Some(token);
        Ok(())
    }

    pub fn selected_for(&self, chain: u64) -> Result<Association, RegistryError> {
        self.ensure_open()?;
        let selected = self.selected.ok_or(RegistryError::NotSelected)?;
        if selected.association.chain != chain {
            return Err(RegistryError::NotSelected);
        }
        Ok(selected.association)
    }

    /// Native owner must retire before releasing its retained COM identities.
    /// Selected retirement is terminal: later observations cannot retarget capture.
    pub fn retire(&mut self, token: AssociationToken) -> Result<(), RegistryError> {
        self.ensure_open()?;
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| **entry == Some(token))
            .ok_or(RegistryError::Stale)?;
        *slot = None;
        if self.selected == Some(token) {
            self.close();
        }
        Ok(())
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.selected = None;
        self.entries.fill(None);
    }

    fn ensure_open(&self) -> Result<(), RegistryError> {
        if self.closed {
            Err(RegistryError::Closed)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn association(n: u64) -> Association {
        Association {
            chain: n,
            queue: 100 + n,
            device: 200,
            window: 300 + n,
        }
    }

    #[test]
    fn explicit_selection_never_follows_unrelated_observation() {
        let mut registry = QueueRegistry::default();
        let first = registry.observe(association(1)).unwrap();
        let second = registry.observe(association(2)).unwrap();
        assert_eq!(registry.selected_for(1), Err(RegistryError::NotSelected));
        registry.select(second).unwrap();
        assert_eq!(registry.selected_for(2), Ok(association(2)));
        assert_eq!(registry.selected_for(1), Err(RegistryError::NotSelected));
        assert_eq!(registry.select(first), Err(RegistryError::AlreadySelected));
        registry.retire(first).unwrap();
        assert_eq!(registry.selected_for(2), Ok(association(2)));
    }

    #[test]
    fn duplicate_observation_and_selection_are_idempotent() {
        let mut registry = QueueRegistry::default();
        let token = registry.observe(association(1)).unwrap();
        for _ in 0..100 {
            assert_eq!(registry.observe(association(1)), Ok(token));
        }
        registry.select(token).unwrap();
        registry.select(token).unwrap();
    }

    #[test]
    fn rejects_zero_identities_without_consuming_slots() {
        let mut registry = QueueRegistry::default();
        for index in 0..4 {
            let mut fields = [1, 2, 3, 4];
            fields[index] = 0;
            assert_eq!(
                registry.observe(Association {
                    chain: fields[0],
                    queue: fields[1],
                    device: fields[2],
                    window: fields[3]
                }),
                Err(RegistryError::InvalidIdentity)
            );
        }
        assert_eq!(registry.observe(association(1)).unwrap().generation, 1);
    }

    #[test]
    fn conflicts_never_replace_an_authoritative_association() {
        let mut registry = QueueRegistry::default();
        let a = association(1);
        let token = registry.observe(a).unwrap();
        for conflict in [
            Association { queue: 999, ..a },
            Association { chain: 999, ..a },
            Association { device: 999, ..a },
            Association { window: 999, ..a },
        ] {
            assert_eq!(registry.observe(conflict), Err(RegistryError::Conflict));
        }
        registry.select(token).unwrap();
        assert_eq!(registry.selected_for(a.chain), Ok(a));
        // Multiple windows may legitimately use one DIRECT queue on one device.
        registry
            .observe(Association {
                queue: a.queue,
                ..association(2)
            })
            .unwrap();
        assert_eq!(
            registry.observe(Association {
                queue: a.queue,
                device: 999,
                ..association(3)
            }),
            Err(RegistryError::Conflict)
        );
    }

    #[test]
    fn full_registry_does_not_evict_and_retirement_reuses_capacity() {
        let mut registry = QueueRegistry::default();
        let first = registry.observe(association(1)).unwrap();
        for n in 2..=ASSOCIATION_CAPACITY as u64 {
            registry.observe(association(n)).unwrap();
        }
        assert_eq!(registry.observe(association(99)), Err(RegistryError::Full));
        assert_eq!(registry.observe(association(1)), Ok(first));
        registry.retire(first).unwrap();
        registry.observe(association(99)).unwrap();
        assert_eq!(registry.select(first), Err(RegistryError::Stale));
    }

    #[test]
    fn reused_addresses_cannot_satisfy_retired_tokens() {
        let mut registry = QueueRegistry::default();
        let old = registry.observe(association(1)).unwrap();
        registry.retire(old).unwrap();
        let new = registry.observe(association(1)).unwrap();
        assert_ne!(old, new);
        assert_eq!(registry.retire(old), Err(RegistryError::Stale));
        assert_eq!(registry.select(old), Err(RegistryError::Stale));
        registry.select(new).unwrap();
    }

    #[test]
    fn selected_retirement_and_close_are_terminal() {
        let mut registry = QueueRegistry::default();
        let selected = registry.observe(association(1)).unwrap();
        let unrelated = registry.observe(association(2)).unwrap();
        registry.select(selected).unwrap();
        registry.retire(selected).unwrap();
        registry.close();
        assert_eq!(registry.selected_for(1), Err(RegistryError::Closed));
        assert_eq!(registry.observe(association(1)), Err(RegistryError::Closed));
        assert_eq!(registry.select(unrelated), Err(RegistryError::Closed));
        assert_eq!(registry.retire(unrelated), Err(RegistryError::Closed));
    }

    #[test]
    fn generation_exhaustion_never_wraps_or_reuses_an_old_token() {
        let mut registry = QueueRegistry {
            next_generation: u64::MAX,
            ..Default::default()
        };
        assert_eq!(
            registry.observe(association(1)),
            Err(RegistryError::GenerationExhausted)
        );
        assert!(registry.entries.iter().all(Option::is_none));
    }
}
