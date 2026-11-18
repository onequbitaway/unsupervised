//! `say` and `post`: speech that viewers see.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Audience, Begin, Builder, EventMeta, VerbSpec};

use super::args::TextArgs;
use super::meter::limited;
use crate::params::{self, Params};
use crate::social::Channel;
use crate::{TALK_ALLOWED_IN, spec, text};

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.instant_verb::<TextArgs>(
        VerbSpec {
            free: true,
            allowed_in: TALK_ALLOWED_IN,
            ..spec("say", "Say something aloud. The residents near you hear it.")
        },
        move |realm, ctx, args| {
            text::check_text(&args.text, realm.param(p.say_text_max), "text")?;
            limited(realm, ctx, Channel::Say, p.says_per_hour, || {
                let pos = realm.pos(ctx.idx);
                let radius = params::span(realm, p.say_radius);
                realm.emit(
                    EventMeta::public("society.said", params::drama(realm, p.drama_said))
                        .actor(ctx.resident)
                        .at(pos)
                        .audience(Audience::Nearby(radius)),
                    &json!({ "text": text::untrusted(ctx.resident, &args.text) }),
                );
                // The speaker is in the list of the kernel; it does not hear
                // itself. The listeners on the other side of a district
                // border get the notice in the next frame.
                let here = realm.nearby(pos, radius).len().saturating_sub(1);
                let across = realm
                    .nearby_outside(pos, radius)
                    .iter()
                    .filter(|other| realm.idx_of(other.id).is_none())
                    .count();
                let heard_by = here + across;
                Ok(json!({ "heard_by": heard_by }))
            })
        },
    );
    b.verb::<TextArgs>(
        spec("post", "Write an entry on the public feed. Viewers see it. It takes a few minutes."),
        move |realm, ctx, args| {
            text::check_text(&args.text, realm.param(p.post_text_max), "text")?;
            limited(realm, ctx, Channel::Post, p.posts_per_day, || {
                Ok(Begin::Busy { ticks: params::span(realm, p.post_ticks) })
            })
        },
        move |realm, ctx, args| {
            let post = realm.alloc_id();
            realm.emit(
                EventMeta::public("society.posted", params::drama(realm, p.drama_posted))
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({ "post": post, "text": text::untrusted(ctx.resident, &args.text) }),
            );
            Ok(json!({ "post": post }))
        },
    );
}
