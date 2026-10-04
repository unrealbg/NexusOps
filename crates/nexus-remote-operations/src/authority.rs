use nexus_model::{HostId, HostSessionId, OperationRisk};
use std::{any::Any, marker::PhantomData};
use tokio::time::Instant;
use uuid::Uuid;

/// Opaque, unpredictable identity for one memory-only approval authority.
/// This native type deliberately has no serde or TypeScript representation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RemoteOperationPlanId(Uuid);

impl RemoteOperationPlanId {
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Smallest connection binding that identifies the authenticated backend session.
/// Host configuration and credentials cannot change while this identity remains current.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorityBinding {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub generation: u64,
}

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A reviewed semantic operation implemented inside this crate.
///
/// Goal 05A intentionally provides no production implementation. Future milestones
/// must add a concrete native type here rather than accepting command text.
pub trait NativeOperation: sealed::Sealed + Send + Sync + 'static {
    type Target: Send + Sync + 'static;
    type Preconditions: Send + Sync + 'static;
    type Payload: Send + Sync + 'static;

    const RISK: OperationRisk;
}

pub struct PlanDraft<O: NativeOperation> {
    pub(crate) binding: AuthorityBinding,
    pub(crate) target: O::Target,
    pub(crate) preconditions: O::Preconditions,
    pub(crate) payload: O::Payload,
    marker: PhantomData<O>,
}

impl<O: NativeOperation> PlanDraft<O> {
    pub fn new(
        binding: AuthorityBinding,
        target: O::Target,
        preconditions: O::Preconditions,
        payload: O::Payload,
    ) -> Self {
        Self {
            binding,
            target,
            preconditions,
            payload,
            marker: PhantomData,
        }
    }
}

struct AuthorityBody<O: NativeOperation> {
    target: O::Target,
    preconditions: O::Preconditions,
    payload: O::Payload,
}

pub(crate) struct StoredAuthority {
    pub id: RemoteOperationPlanId,
    pub binding: AuthorityBinding,
    pub operation_type: std::any::TypeId,
    pub risk: OperationRisk,
    pub issued_at: Instant,
    pub expires_at: Instant,
    body: Box<dyn Any + Send + Sync>,
}

impl StoredAuthority {
    pub(crate) fn new<O: NativeOperation>(
        id: RemoteOperationPlanId,
        draft: PlanDraft<O>,
        issued_at: Instant,
        expires_at: Instant,
    ) -> Self {
        Self {
            id,
            binding: draft.binding,
            operation_type: std::any::TypeId::of::<O>(),
            risk: O::RISK,
            issued_at,
            expires_at,
            body: Box::new(AuthorityBody::<O> {
                target: draft.target,
                preconditions: draft.preconditions,
                payload: draft.payload,
            }),
        }
    }

    pub(crate) fn into_typed<O: NativeOperation>(self) -> Option<ConsumedAuthority<O>> {
        let body = self.body.downcast::<AuthorityBody<O>>().ok()?;
        Some(ConsumedAuthority {
            id: self.id,
            binding: self.binding,
            risk: self.risk,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
            target: body.target,
            preconditions: body.preconditions,
            payload: body.payload,
            marker: PhantomData,
        })
    }
}

/// Authority moved permanently out of the pending store.
pub struct ConsumedAuthority<O: NativeOperation> {
    id: RemoteOperationPlanId,
    binding: AuthorityBinding,
    risk: OperationRisk,
    issued_at: Instant,
    expires_at: Instant,
    target: O::Target,
    preconditions: O::Preconditions,
    payload: O::Payload,
    marker: PhantomData<O>,
}

impl<O: NativeOperation> ConsumedAuthority<O> {
    pub fn id(&self) -> RemoteOperationPlanId {
        self.id
    }

    pub fn binding(&self) -> AuthorityBinding {
        self.binding
    }

    pub fn risk(&self) -> OperationRisk {
        self.risk
    }

    pub fn target(&self) -> &O::Target {
        &self.target
    }

    pub fn preconditions(&self) -> &O::Preconditions {
        &self.preconditions
    }

    pub fn payload(&self) -> &O::Payload {
        &self.payload
    }

    pub fn issued_at(&self) -> Instant {
        self.issued_at
    }

    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanReceipt {
    pub id: RemoteOperationPlanId,
    pub binding: AuthorityBinding,
    pub risk: OperationRisk,
    pub issued_at: Instant,
    pub expires_at: Instant,
    pub superseded: Option<RemoteOperationPlanId>,
}
