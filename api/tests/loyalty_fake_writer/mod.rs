//! An in-process `LoyaltyRuleWriter` for tests that drive the LY-6 commands and the approval
//! subject directly (no HTTP fake). Each call answers from a script the test sets; every
//! `approve` and `reject` is recorded. Shared by the #2881 tests.

#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};
use uuid::Uuid;
use xc_loyalty::application::rule_publisher::{
    LoyaltyRuleError, LoyaltyRuleWriter, RuleKind, RuleVersion,
};

/// The same type as `futures::future::BoxFuture`, which the port's methods return.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Default)]
struct Script {
    status: String,
    approved_ref: Option<String>,
    create: Option<LoyaltyRuleError>,
    get: Option<LoyaltyRuleError>,
    submit: Option<LoyaltyRuleError>,
    approve: Option<LoyaltyRuleError>,
    reject: Option<LoyaltyRuleError>,
    approved_refs: Vec<String>,
    reject_reasons: Vec<String>,
}

/// One rule version, `id`, of kind POINT; `status` starts at `DRAFT`.
#[derive(Clone)]
pub struct FakeWriter {
    pub id: Uuid,
    script: Arc<Mutex<Script>>,
}

impl FakeWriter {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            script: Arc::new(Mutex::new(Script {
                status: "DRAFT".to_owned(),
                ..Script::default()
            })),
        }
    }

    pub fn as_writer(&self) -> Arc<dyn LoyaltyRuleWriter> {
        Arc::new(self.clone())
    }

    pub fn object_id(&self) -> String {
        format!("POINT/{}", self.id)
    }

    pub fn fail_create(&self, e: LoyaltyRuleError) {
        self.script.lock().unwrap().create = Some(e);
    }
    pub fn fail_get(&self, e: LoyaltyRuleError) {
        self.script.lock().unwrap().get = Some(e);
    }
    pub fn fail_submit(&self, e: LoyaltyRuleError) {
        self.script.lock().unwrap().submit = Some(e);
    }
    pub fn fail_approve(&self, e: LoyaltyRuleError) {
        self.script.lock().unwrap().approve = Some(e);
    }

    /// Every `approved_ref` passed to `approve`, in call order.
    pub fn approved_refs(&self) -> Vec<String> {
        self.script.lock().unwrap().approved_refs.clone()
    }

    /// Every `reason` passed to `reject`, in call order.
    pub fn reject_reasons(&self) -> Vec<String> {
        self.script.lock().unwrap().reject_reasons.clone()
    }

    fn version(&self, s: &Script) -> RuleVersion {
        RuleVersion {
            id: self.id,
            kind: RuleKind::Point,
            status: s.status.clone(),
            approved_ref: s.approved_ref.clone(),
            content: json!({ "pointsRatio": "1", "minimumBasisAmount": "100" }),
        }
    }
}

impl LoyaltyRuleWriter for FakeWriter {
    fn create_draft<'a>(
        &'a self,
        _kind: RuleKind,
        _fields: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        Box::pin(async move {
            let s = self.script.lock().unwrap();
            match &s.create {
                Some(e) => Err(e.clone()),
                None => Ok(self.version(&s)),
            }
        })
    }

    fn submit<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        Box::pin(async move {
            let mut s = self.script.lock().unwrap();
            if let Some(e) = &s.submit {
                return Err(e.clone());
            }
            s.status = "PENDING_APPROVAL".to_owned();
            Ok(self.version(&s))
        })
    }

    fn approve<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
        approved_ref: &'a str,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        Box::pin(async move {
            let mut s = self.script.lock().unwrap();
            s.approved_refs.push(approved_ref.to_owned());
            if let Some(e) = &s.approve {
                return Err(e.clone());
            }
            s.status = "APPROVED".to_owned();
            s.approved_ref = Some(approved_ref.to_owned());
            Ok(self.version(&s))
        })
    }

    fn reject<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        Box::pin(async move {
            let mut s = self.script.lock().unwrap();
            s.reject_reasons.push(reason.to_owned());
            if let Some(e) = &s.reject {
                return Err(e.clone());
            }
            s.status = "REJECTED".to_owned();
            Ok(self.version(&s))
        })
    }

    fn get<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<RuleVersion>, LoyaltyRuleError>> {
        Box::pin(async move {
            let s = self.script.lock().unwrap();
            if let Some(e) = &s.get {
                return Err(e.clone());
            }
            if kind != RuleKind::Point || id != self.id {
                return Ok(None);
            }
            Ok(Some(self.version(&s)))
        })
    }

    fn list<'a>(
        &'a self,
        _kind: RuleKind,
    ) -> BoxFuture<'a, Result<Vec<RuleVersion>, LoyaltyRuleError>> {
        Box::pin(async move {
            let s = self.script.lock().unwrap();
            Ok(vec![self.version(&s)])
        })
    }
}
