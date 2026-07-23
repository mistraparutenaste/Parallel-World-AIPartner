use parallel_world_desktop::behavior::{
    ProactiveDeliveryDecision, ProactiveDeliveryInput, decide_proactive_delivery,
};

fn input() -> ProactiveDeliveryInput {
    ProactiveDeliveryInput {
        master_enabled: true,
        profile_enabled: true,
        trigger_enabled: true,
        in_quiet_hours: false,
        temporary_conversation: false,
        evaluator_approved: true,
        generated_text: "少し休憩する？".to_owned(),
        lease_cancelled: false,
    }
}

#[test]
fn proactive_delivery_requires_every_privacy_and_policy_gate() {
    assert_eq!(
        decide_proactive_delivery(&input()),
        ProactiveDeliveryDecision::Deliver("少し休憩する？".to_owned())
    );

    let mut blocked = input();
    blocked.temporary_conversation = true;
    assert_eq!(
        decide_proactive_delivery(&blocked),
        ProactiveDeliveryDecision::Skip
    );

    blocked = input();
    blocked.lease_cancelled = true;
    assert_eq!(
        decide_proactive_delivery(&blocked),
        ProactiveDeliveryDecision::Skip
    );
}

#[test]
fn proactive_delivery_rejects_empty_or_unbounded_generated_text() {
    let mut empty = input();
    empty.generated_text = "  ".to_owned();
    assert_eq!(
        decide_proactive_delivery(&empty),
        ProactiveDeliveryDecision::Skip
    );

    let mut oversized = input();
    oversized.generated_text = "あ".repeat(501);
    assert_eq!(
        decide_proactive_delivery(&oversized),
        ProactiveDeliveryDecision::Skip
    );
}
