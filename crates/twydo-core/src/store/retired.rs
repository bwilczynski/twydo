//! Restoring an archived local snapshot using the existing gated move planners.
use super::*;

impl Store {
    /// Recover a whole local snapshot into an existing list. The archive retains the
    /// exact operation, so retrying a confirmed recovery reuses its identities.
    /// Returns (operation id, whether this call wrote).
    pub fn recover_list(
        &mut self,
        active: &Snapshot,
        backup_id: &str,
        to: &str,
        now: DateTime<Utc>,
    ) -> Result<(String, bool), WriteError> {
        let _lock = self.lock()?;
        self.check_not_stuck()?;
        if !crate::retired::plain(to) || !active.collections.iter().any(|c| c.id == to) {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                "choose an existing destination list",
            ));
        }
        let dest = self.vdir.join(to);
        let metadata = fs::symlink_metadata(&dest)?;
        if !metadata.is_dir() || metadata.is_symlink() {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                "destination must be a plain list directory",
            ));
        }
        let mut b = crate::retired::read(&self.state, backup_id)?;
        if !b.retired {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                "list retirement has not finished",
            ));
        }
        if b.collection == to {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                "recover into a different existing list",
            ));
        }
        if let Some(entry) = b.recovery.clone() {
            if b.destination.as_deref() != Some(to) {
                return Err(io_error(
                    io::ErrorKind::InvalidInput,
                    "this backup already has a recovery destination",
                ));
            }
            if entry.writes.iter().all(|w| {
                fs::read(self.vdir.join(&w.path)).ok().as_deref()
                    == w.after.as_ref().map(String::as_bytes)
            }) {
                return Ok((entry.id, false));
            }
            // After undo, use ordinary redo. This avoids resurrecting tasks removed or
            // edited since recovery by interpreting their absence as a fresh import.
            if self
                .journal
                .get(&entry.id)?
                .is_some_and(|e| e.stage == Stage::Applied)
            {
                return Err(io_error(
                    io::ErrorKind::InvalidInput,
                    "this backup was recovered already; use redo after undo, or inspect its changed tasks",
                ));
            }
            for w in &entry.writes {
                self.check_holds(&w.path, w.before.as_deref())?;
            }
            let id = self.run(entry)?;
            self.done.push(super::Step::Applied(id.clone()));
            self.undone.clear();
            return Ok((id, true));
        }
        let mut source = crate::retired::snapshot(&active.config, backup_id)?;
        source.collections.extend(
            active
                .collections
                .iter()
                .filter(|c| c.id != b.collection)
                .cloned(),
        );
        if !source.errors.is_empty() || source.tasks.len() != b.tasks {
            return Err(io_error(
                io::ErrorKind::InvalidData,
                "backup contains unreadable or unsupported task resources",
            ));
        }
        let today = now.with_timezone(&source.tz()).date_naive();
        let mut writes = Vec::new();
        let mut mapping = std::collections::BTreeMap::new();
        for i in 0..source.tasks.len() {
            if source.parent(i).is_some() {
                continue;
            }
            let op = Op::Move {
                uid: source.tasks[i].uid.clone(),
                to: to.into(),
            };
            gate::check(&source, &self.matrix, self.mode, &op, today)
                .map_err(WriteError::Blocked)?;
            let plan = self.plan(&source, &op, now)?;
            for pair in plan.uids.as_chunks::<2>().0 {
                mapping.insert(pair[0].clone(), pair[1].clone());
            }
            writes.extend(
                plan.writes
                    .into_iter()
                    .filter(|w| crate::retired::in_collection(&w.path, to)),
            );
        }
        if writes.len() != b.tasks {
            return Err(io_error(
                io::ErrorKind::InvalidData,
                "backup family cannot be recovered as one complete set",
            ));
        }
        // References between archived families/history are remapped with the same
        // known UID/parentuid/RUID fields used by the existing move implementation.
        for w in &mut writes {
            let text = w.after.as_ref().expect("move destination");
            let mut cal = ical::parse(text.as_bytes())
                .map_err(|e| io_error(io::ErrorKind::InvalidData, e.to_string()))?;
            for todo in cal.root.children_named_mut("VTODO") {
                if let Some(p) = todo.prop_mut("RELATED-TO")
                    && let Some(new) = mapping.get(&p.value)
                {
                    p.set_value(new);
                }
                if let Some(p) = todo.prop_mut("X-2DOAPP-METADATA")
                    && let Some(mut meta) = MetaJson::parse(&p.value)
                {
                    let old = meta.clone();
                    for key in ["uid", "parentuid", "RUID"] {
                        if let Some(new) =
                            meta.get_str(key).and_then(|uid| mapping.get(uid)).cloned()
                        {
                            meta.set_str(key, &new);
                        }
                    }
                    if old != meta {
                        p.set_value(&meta.encode());
                    }
                }
            }
            w.after = Some(
                String::from_utf8(ical::serialize(&cal))
                    .map_err(|e| io_error(io::ErrorKind::InvalidData, e.to_string()))?,
            );
            self.check_holds(&w.path, None)?;
        }
        let entry = Entry {
            format: FORMAT,
            id: new_op_id(now),
            created: now,
            kind: "recover".into(),
            uids: mapping.values().cloned().collect(),
            stage: Stage::Prepared,
            inverse_of: None,
            from_queue: None,
            synced: None,
            writes,
            holds: Vec::new(),
            family: None,
            families: Vec::new(),
        };
        b.destination = Some(to.into());
        b.recovery = Some(entry.clone());
        crate::retired::save(&self.state, &b)?;
        let id = self.run(entry)?;
        self.done.push(super::Step::Applied(id.clone()));
        self.undone.clear();
        Ok((id, true))
    }
}
