//! The Chief's inbox: what the next chat-lane turn is built from.
//!
//! This is the fix for the Orpheus steering problem. There, a user message to a
//! busy coordinator became a steering message that a background wake could run
//! ahead of, so the Chief answered late or not at all. Here every pending item
//! is drained into one turn ordered by priority, so an unread user message
//! always leads and a coincident wake rides behind it in the same turn instead
//! of racing ahead in its own.
//!
//! Pure: the actor owns the inbox and decides when to drain it. Control
//! commands (pause/stop/redirect) do not pass through here — they are applied
//! immediately by the actor, ahead of any chief turn (see [`super::control`]).

/// Priority of a chief-facing item. The derived `Ord` ranks by declaration
/// order, so [`Priority::User`] is the smallest and a priority sort (ascending)
/// puts user messages first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    User,
    ClerkFlag,
    LaneResult,
    Wake,
    CheckIn,
}

/// Something the Chief should act on.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A message the user typed.
    User(String),
    /// The Clerk flagged a work lane. The Clerk reaches only the Chief.
    ClerkFlag { avatar: String, severity: String, note: String },
    /// A lane submitted a result worth the Chief's notice.
    LaneResult { avatar: String, summary: String },
    /// A batch settled or needs attention (from the reconciler).
    Wake { work_id: String },
    /// A slow check-in while a batch runs healthily.
    CheckIn { work_id: String },
}

impl Item {
    pub fn priority(&self) -> Priority {
        match self {
            Item::User(_) => Priority::User,
            Item::ClerkFlag { .. } => Priority::ClerkFlag,
            Item::LaneResult { .. } => Priority::LaneResult,
            Item::Wake { .. } => Priority::Wake,
            Item::CheckIn { .. } => Priority::CheckIn,
        }
    }

    /// Wakes and check-ins coalesce per work item, keyed by (kind, work_id), so
    /// repeated reconciler ticks for one unchanged batch don't pile up. A
    /// `Wake` and a `CheckIn` for the same work are different kinds and both
    /// survive. Everything else never coalesces.
    fn coalesce_tag(&self) -> Option<(u8, &str)> {
        match self {
            Item::Wake { work_id } => Some((0, work_id)),
            Item::CheckIn { work_id } => Some((1, work_id)),
            _ => None,
        }
    }
}

struct Entry {
    seq: u64,
    item: Item,
}

/// The Chief's pending work, cleared one turn at a time.
#[derive(Default)]
pub struct Inbox {
    next_seq: u64,
    pending: Vec<Entry>,
}

impl Inbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an item. A wake or check-in whose coalesce tag already waits
    /// undelivered is dropped.
    pub fn push(&mut self, item: Item) {
        if let Some(tag) = item.coalesce_tag() {
            if self.pending.iter().any(|e| e.item.coalesce_tag() == Some(tag)) {
                return;
            }
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.pending.push(Entry { seq, item });
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// True when an unread user message is waiting. The actor uses this to
    /// preempt a running wake (it injects the user message into the Chief's
    /// current turn rather than waiting for the next one).
    pub fn has_user(&self) -> bool {
        self.pending.iter().any(|e| matches!(e.item, Item::User(_)))
    }

    /// Take everything pending as one turn's worth of items, highest priority
    /// first, ties broken by arrival order. The inbox is left empty; these
    /// items are now delivered.
    pub fn drain(&mut self) -> Vec<Item> {
        let mut entries = std::mem::take(&mut self.pending);
        entries.sort_by(|a, b| a.item.priority().cmp(&b.item.priority()).then(a.seq.cmp(&b.seq)));
        entries.into_iter().map(|e| e.item).collect()
    }
}
