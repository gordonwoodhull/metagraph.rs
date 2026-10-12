//! The transaction's pattern-independent pieces (M4): the token that
//! stamps provisional refs, the ref type itself, and the handle→ref
//! mapping trait. Everything pattern-specific — the `Tx` type, its op
//! queue, staging methods, and `flush` — is emitted per `implement!`
//! call by the macro; this module holds only what is identical for every
//! store.
//!
//! Staging never touches a store: the change set is a queue, not a
//! private clone. [`Ref`] names an entity inside that change set — a new
//! one by submission order ([`RefKind::Pending`] carries the op index),
//! an old one by id ([`RefKind::Existing`]). Refs are opaque tokens with
//! no read traits; they resolve to real records only inside `flush`.

use std::sync::atomic::{AtomicU64, Ordering};

/// Transaction token, minted once per `edit()`/`into_edit()` and stamped
/// onto every `Pending` ref. After a flush the token is never minted
/// again, so a ref left over from a dead transaction can never match a
/// live one. `TxToken(0)` is the sentinel stamped onto `Existing` refs
/// minted from raw ids (which carry no transaction provenance).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TxToken(pub u64);

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Mint a fresh transaction token (starts past the `Existing` sentinel).
#[doc(hidden)]
pub fn mint_token() -> TxToken {
    TxToken(NEXT_TOKEN.fetch_add(1, Ordering::Relaxed))
}

/// The unstamped sentinel for `Existing` refs minted from raw ids.
#[doc(hidden)]
pub const UNSTAMPED: TxToken = TxToken(0);

/// `Existing(old-store id) | Pending(change-set op index)`. Op indices
/// count *every* queued op, removals included, so a `Pending` ref is
/// meaningful only against the one queue it was minted from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefKind<I> {
    Existing(I),
    Pending(u32),
}

/// A provisional ref: an opaque, `Copy` token naming an entity inside one
/// change set. No read traits — reads happen through handles against a
/// real store; a ref becomes a record only when `flush` replays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ref<I> {
    /// The transaction that minted a `Pending` ref (`UNSTAMPED` on an
    /// `Existing` ref minted from a raw id or handle).
    pub token: TxToken,
    pub kind: RefKind<I>,
}

impl<I> Ref<I> {
    /// An `Existing` ref from an old-store id (unstamped: raw ids carry
    /// no transaction provenance; the id is validated at flush).
    pub fn existing(id: I) -> Self {
        Ref { token: UNSTAMPED, kind: RefKind::Existing(id) }
    }

    /// Eager stamp check at a staging call: only `Pending` refs carry a
    /// meaningful stamp, and mis-wiring one (an op index into another
    /// transaction's queue) is the silent, catastrophic case, so it is
    /// caught where it happens — a staging panic, like a foreign handle.
    pub fn check_stamp(&self, tx: TxToken) {
        if let RefKind::Pending(_) = self.kind {
            assert!(
                self.token == tx,
                "Pending ref stamped with a different transaction token: \
                 it was minted by another transaction or is stale after a flush"
            );
        }
    }
}

/// Handle→ref mapping (M4's D7 pin): the transaction converts old-store
/// handles to `Existing` refs. One marker-generic trait in the runtime
/// crate — an algorithm-crate declaration would not unify across crates
/// in a bound web — with plain associated types; `implement!` emits one
/// impl per entity on the transaction, marked by that entity's
/// per-entity marker. The impl asserts (a hard `assert!`) the handle's
/// store pointer against the transaction's own store: a foreign handle's
/// id would silently resolve against the wrong arena.
pub trait HandleToRef<M> {
    /// The old-store handle type (the read side's associated entity).
    type Handle;
    /// The transaction's ref type for that entity.
    type Ref;
    fn to_ref(&self, handle: Self::Handle) -> Self::Ref;
}
