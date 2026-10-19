//! The claim line and the watch list are cursors over numbered slots. These
//! tests drive `queue_front` and `watch_step` one call at a time over many
//! layouts and compare every position and slot with a plain model of the
//! same rules, plus the submit and release paths that move the line.

use soroban_sdk::{Address, BytesN, Env};
use std::vec::Vec;

use super::common::*;
use crate::types::ClaimStatus;

const TIER_C: u32 = 3;

fn file(env: &Env, s: &Setup<'_>, wallet: &Address, seed: u8, entitlement: i128) -> BytesN<32> {
    submit_claim_signed(env, s, &s.oracle, wallet, &tx_hash(env, seed), &entitlement, &TIER_C, &now_ts(env))
}

/// Small deterministic generator so every layout is reproducible.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn id(env: &Env, a: u8, b: u8) -> BytesN<32> {
    let mut x = [0u8; 32];
    x[0] = 0xEE;
    x[1] = a;
    x[2] = b;
    BytesN::from_array(env, &x)
}

// -----------------------------------------------------------------------
// watch list
// -----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum W {
    Hole,
    Done,
    Live,
}

/// The watch rules, on plain values.
fn model_watch(slots: &mut [W], head: &mut u64, cursor: &mut u64, tail: u64) -> bool {
    if *head >= tail {
        return false;
    }
    let mut cur = *cursor;
    if cur < *head || cur >= tail {
        cur = *head;
    }
    let done = match slots[cur as usize] {
        W::Hole => true,
        W::Done => {
            slots[cur as usize] = W::Hole;
            true
        }
        W::Live => false,
    };
    if done && cur == *head {
        *head += 1;
        let mut skipped = 0;
        while *head < tail && skipped < 8 && slots[*head as usize] == W::Hole {
            *head += 1;
            skipped += 1;
        }
    }
    *cursor = cur + 1;
    true
}

#[test]
fn the_watch_list_moves_exactly_as_the_model_says() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    // A fresh claim is far from any deadline: watching it changes nothing.
    let live = file(&env, &s, &w, 1, MIN_STAKE);
    assert_eq!(s.client.get_claim(&live).unwrap().status, ClaimStatus::AwaitingApproval);

    let mut rng = Lcg(7);
    for trial in 0..60u8 {
        let n = 1 + rng.next(20) as usize;
        let mut slots: Vec<W> = (0..n)
            .map(|_| match rng.next(10) {
                0..=3 => W::Hole,
                4..=6 => W::Done,
                _ => W::Live,
            })
            .collect();
        if trial % 4 == 0 && n > 2 {
            // A run of holes longer than one step may skip.
            for x in slots.iter_mut().skip(1).take(n - 2) {
                *x = W::Hole;
            }
        }
        let mut head = rng.next(n as u64);
        for x in slots.iter_mut().take(head as usize) {
            *x = W::Hole;
        }
        let tail = n as u64;
        let mut cursor = rng.next(tail + 3);

        env.as_contract(&s.contract_id, || {
            for seq in 0..32u64 {
                crate::storage::remove_watch_at(&env, seq);
            }
            for (i, x) in slots.iter().enumerate() {
                match x {
                    W::Hole => {}
                    W::Done => crate::storage::set_watch_at(&env, i as u64, &id(&env, trial, i as u8)),
                    W::Live => crate::storage::set_watch_at(&env, i as u64, &live),
                }
            }
            crate::storage::set_watch_head(&env, head);
            crate::storage::set_watch_tail(&env, tail);
            crate::storage::set_watch_cursor(&env, cursor);

            for step in 0..(3 * n + 6) {
                let want = model_watch(&mut slots, &mut head, &mut cursor, tail);
                let got = crate::claim::watch_step(&env);
                let ctx = (trial, step);
                assert_eq!(got, want, "return {:?}", ctx);
                assert_eq!(crate::storage::get_watch_head(&env), head, "head {:?}", ctx);
                assert_eq!(crate::storage::get_watch_cursor(&env), cursor, "cursor {:?}", ctx);
                assert_eq!(crate::storage::get_watch_tail(&env), tail, "tail {:?}", ctx);
                for (i, x) in slots.iter().enumerate() {
                    let present = crate::storage::get_watch_at(&env, i as u64).is_some();
                    assert_eq!(present, *x != W::Hole, "slot {} {:?}", i, ctx);
                }
                if !want {
                    break;
                }
            }
        });
    }
}

// -----------------------------------------------------------------------
// claim line
// -----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Q {
    Hole,
    /// The slot names a claim that does not exist.
    Missing,
    /// A Reserved claim whose wallet's slot holds a different claim.
    DeadReserved,
    /// No longer Reserved (taken over by an override).
    DeadOther,
    Live,
}

/// The line rules, on plain values. Returns the slot now first, if any.
fn model_front(slots: &mut [Q], head: &mut u64, tail: u64) -> Option<usize> {
    let mut skipped = 0;
    while *head < tail && skipped < 16 {
        let i = *head as usize;
        match slots[i] {
            Q::Hole => {
                *head += 1;
                skipped += 1;
                continue;
            }
            Q::Live => return Some(i),
            _ => slots[i] = Q::Hole,
        }
        *head += 1;
        skipped += 1;
    }
    None
}

#[test]
fn the_claim_line_moves_exactly_as_the_model_says() {
    let env = new_env();
    let s = setup(&env);
    let (w_live, _) = staked_wallet(&env, &s);
    let (w_other, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let live = file(&env, &s, &w_live, 1, expected_stress_cap(&s) + 1);
    let template = s.client.get_claim(&live).unwrap();
    assert_eq!(template.status, ClaimStatus::Reserved);
    // w_other's own slot holds some other claim, so its stake is live but
    // any queued claim of its is dead.
    let others_claim = id(&env, 0xFF, 0xFF);

    let mut rng = Lcg(11);
    for trial in 0..60u8 {
        let n = 1 + rng.next(30) as usize;
        let mut slots: Vec<Q> = (0..n)
            .map(|_| match rng.next(12) {
                0..=2 => Q::Hole,
                3..=4 => Q::Missing,
                5..=7 => Q::DeadReserved,
                8..=9 => Q::DeadOther,
                _ => Q::Live,
            })
            .collect();
        let mut head = 0u64;
        let tail = n as u64;
        let ids: Vec<BytesN<32>> = (0..n).map(|i| id(&env, trial, i as u8)).collect();

        env.as_contract(&s.contract_id, || {
            for seq in 0..64u64 {
                crate::storage::remove_queue_at(&env, seq);
            }
            let mut r = crate::storage::get_stake(&env, &w_other).unwrap();
            r.reserved_claim_id = Some(others_claim.clone());
            crate::storage::set_stake(&env, &w_other, &r);
            for (i, x) in slots.iter().enumerate() {
                let slot_id = match x {
                    Q::Hole => continue,
                    Q::Live => live.clone(),
                    _ => ids[i].clone(),
                };
                let mut c = template.clone();
                c.wallet = w_other.clone();
                match x {
                    Q::DeadReserved => crate::storage::set_claim(&env, &slot_id, &c),
                    Q::DeadOther => {
                        c.status = ClaimStatus::Active;
                        crate::storage::set_claim(&env, &slot_id, &c)
                    }
                    _ => {}
                }
                crate::storage::set_queue_at(&env, i as u64, &slot_id);
            }
            crate::storage::set_queue_head(&env, head);
            crate::storage::set_queue_tail(&env, tail);

            let before = slots.clone();
            for call in 0..4 {
                let want = model_front(&mut slots, &mut head, tail);
                let got = crate::claim::queue_front(&env);
                let ctx = (trial, call);
                assert_eq!(got, want.map(|_| live.clone()), "front {:?}", ctx);
                assert_eq!(crate::storage::get_queue_head(&env), head, "head {:?}", ctx);
                for (i, x) in slots.iter().enumerate() {
                    let present = crate::storage::get_queue_at(&env, i as u64).is_some();
                    assert_eq!(present, *x != Q::Hole, "slot {} {:?}", i, ctx);
                }
                if want.is_some() || head >= tail {
                    break;
                }
            }
            for (i, x) in before.iter().enumerate() {
                let passed = slots[i] == Q::Hole;
                let status = crate::storage::get_claim(&env, &ids[i]).map(|c| c.status);
                match x {
                    Q::DeadReserved if passed => assert_eq!(status, Some(ClaimStatus::Expired)),
                    Q::DeadReserved => assert_eq!(status, Some(ClaimStatus::Reserved)),
                    Q::DeadOther => assert_eq!(status, Some(ClaimStatus::Active)),
                    _ => {}
                }
            }
            // Expiring a dead claim never clears another claim's slot.
            let r = crate::storage::get_stake(&env, &w_other).unwrap();
            assert_eq!(r.reserved_claim_id, Some(others_claim.clone()));
        });
    }
}

#[test]
fn a_claim_that_fits_with_nobody_in_line_never_enters_the_line() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let before = s.client.get_queue();
    let c = file(&env, &s, &w, 1, MIN_STAKE);
    assert_eq!(s.client.get_claim(&c).unwrap().status, ClaimStatus::AwaitingApproval);
    assert_eq!(s.client.get_queue(), before);
}

/// More dead places than one check clears: the line is still not empty, so
/// a new claim joins the back even though it fits.
#[test]
fn a_long_dead_stretch_still_counts_as_people_in_line() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    env.as_contract(&s.contract_id, || {
        for i in 0..17u64 {
            crate::storage::set_queue_at(&env, i, &id(&env, 1, i as u8));
        }
        crate::storage::set_queue_head(&env, 0);
        crate::storage::set_queue_tail(&env, 17);
    });
    let c = file(&env, &s, &w, 1, MIN_STAKE);
    // It took place 17 (tail 18) and, the stretch cleared, was released at once.
    assert_eq!(s.client.get_claim(&c).unwrap().status, ClaimStatus::AwaitingApproval);
    assert_eq!(s.client.get_queue(), (18, 18));
}

/// A release moves the head to just past the released place, then clears
/// dead places from there with a full skip budget.
#[test]
fn a_release_moves_the_head_to_just_past_the_released_place() {
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let c = file(&env, &s, &w, 1, expected_stress_cap(&s) + 1);
    let seq = s.client.get_claim(&c).unwrap().queue_seq;
    env.as_contract(&s.contract_id, || {
        for i in 1..=17u64 {
            crate::storage::set_queue_at(&env, seq + i, &id(&env, 2, i as u8));
        }
        crate::storage::set_queue_tail(&env, seq + 18);
    });
    let mut stakes = 0;
    while s.client.get_claim(&c).unwrap().status == ClaimStatus::Reserved {
        staked_wallet(&env, &s);
        stakes += 1;
        assert!(stakes < 200, "never released");
    }
    assert_eq!(s.client.get_queue(), (seq + 17, seq + 18));
}

#[test]
fn a_dead_claim_in_line_with_a_live_stake_is_expired() {
    // The wallet still has its stake, but its slot was cleared.
    let env = new_env();
    let s = setup(&env);
    let (w, _) = staked_wallet(&env, &s);
    advance_past_time_gate(&env);
    let c = file(&env, &s, &w, 1, expected_stress_cap(&s) + 1);
    env.as_contract(&s.contract_id, || {
        let mut r = crate::storage::get_stake(&env, &w).unwrap();
        r.reserved_claim_id = None;
        crate::storage::set_stake(&env, &w, &r);
        assert!(crate::claim::queue_front(&env).is_none());
    });
    assert_eq!(s.client.get_claim(&c).unwrap().status, ClaimStatus::Expired);
}
