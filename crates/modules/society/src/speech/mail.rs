//! `message` and `org_say`: private text. Viewers do not see it.
//!
//! Both verbs finish at once in the zone. A text for a resident in another
//! zone, and every text for an organization, goes through the global realm:
//! it knows who is alive and who the members are. When it cannot deliver a
//! text, the sender gets a notice.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, Builder, Dest, Message, OrgId, Realm, ResidentId, Scope, VerbSpec,
};

use super::args::{MessageArgs, OrgSayArgs};
use super::meter::limited;
use crate::org::mirror::describe_with;
use crate::org::state::Orgs;
use crate::params::Params;
use crate::social::{Channel, HOUR};
use crate::{TALK_ALLOWED_IN, spec, text};

/// A private message on its way through the global realm.
#[derive(Serialize, Deserialize)]
struct Letter {
    from: ResidentId,
    to: ResidentId,
    text: String,
}

impl Message for Letter {
    const KIND: &'static str = "society.letter";
}

/// A text for the members of an organization on its way to the global
/// realm.
#[derive(Serialize, Deserialize)]
struct Circular {
    from: ResidentId,
    org: OrgId,
    text: String,
}

impl Message for Circular {
    const KIND: &'static str = "society.circular";
}

/// Writes a message to the inbox of its reader.
fn hand_over(realm: &Realm, letter: &Letter) {
    realm.notify(
        letter.to,
        "society.message",
        Some(letter.from),
        json!({ "text": text::untrusted(letter.from, &letter.text) }),
    );
}

/// The global realm sends a message on, or tells the sender that nobody
/// has that id.
fn deliver_letter(realm: &Realm, letter: &Letter) {
    if realm.directory().is_alive(letter.to) {
        hand_over(realm, letter);
    } else {
        realm.notify(
            letter.from,
            "society.message_undelivered",
            None,
            json!({ "to": letter.to, "reason": "no_such_resident" }),
        );
    }
}

fn deliver_circular(realm: &Realm, p: &Params, circular: Circular) {
    let mut orgs = realm.res_mut::<Orgs>();
    let frame = realm.frame();
    let limit = realm.param(p.org_broadcasts_per_hour);
    let refuse = |reason: &str| {
        realm.notify(
            circular.from,
            "society.org_message_undelivered",
            None,
            json!({ "org": circular.org, "reason": reason }),
        );
    };
    let Some(org) = orgs.get_mut(circular.org) else {
        return refuse("no_such_organization");
    };
    if org.role_of(circular.from).is_none() {
        return refuse("not_a_member");
    }
    if !org.said.has_room(frame, HOUR, limit) {
        return refuse("rate_limited");
    }
    org.said.count(frame, HOUR);
    let data =
        describe_with(org, json!({ "text": text::untrusted(circular.from, &circular.text) }));
    for (member, _) in org.members() {
        if member != circular.from {
            realm.notify(member, "society.org_message", Some(circular.from), data.clone());
        }
    }
}

/// Registers the verbs and the handlers of the global realm.
pub(crate) fn build(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Global {
        b.on_message::<Letter>(|realm, _from, letter| deliver_letter(realm, &letter));
        b.on_message::<Circular>(move |realm, _from, circular| {
            deliver_circular(realm, &p, circular);
        });
    }
    b.instant_verb::<MessageArgs>(
        VerbSpec {
            free: true,
            allowed_in: TALK_ALLOWED_IN,
            ..spec("message", "Send a private message to a resident, wherever it is.")
        },
        move |realm, ctx, args| {
            text::check_text(&args.text, realm.param(p.message_text_max), "text")?;
            if args.to == ctx.resident {
                return Err(ActionError::invalid_args("You cannot send a message to yourself."));
            }
            limited(realm, ctx, Channel::Message, p.messages_per_hour, || {
                let here = realm.idx_of(args.to).is_some();
                let letter = Letter { from: ctx.resident, to: args.to, text: args.text.clone() };
                if here {
                    hand_over(realm, &letter);
                } else {
                    realm.send(Dest::Global, &letter);
                }
                Ok(json!({ "to": args.to, "delivered": here }))
            })
        },
    );
    b.instant_verb::<OrgSayArgs>(
        VerbSpec {
            free: true,
            allowed_in: TALK_ALLOWED_IN,
            ..spec("org_say", "Send a text to every other member of an organization of yours.")
        },
        move |realm, ctx, args| {
            text::check_text(&args.text, realm.param(p.message_text_max), "text")?;
            if !realm.memberships(ctx.idx).iter().any(|m| m.org == args.org) {
                return Err(ActionError::not_allowed(format!(
                    "You are not a member of organization {}.",
                    args.org
                )));
            }
            limited(realm, ctx, Channel::OrgSay, p.org_says_per_hour, || {
                let circular =
                    Circular { from: ctx.resident, org: args.org, text: args.text.clone() };
                realm.send(Dest::Global, &circular);
                Ok(json!({ "org": args.org }))
            })
        },
    );
}
