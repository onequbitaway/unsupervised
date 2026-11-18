//! How a subject becomes a shot.

use unsup_protocol::CityId;

use super::score::{Beat, Subject};
use crate::config::DirectorConfig;
use crate::facts::Atlas;
use crate::wire::{Focus, Shot, Zoom};

/// Makes the shot of a subject from the event that speaks for it. The
/// caller sets the number, the frame and the clock.
///
/// Without a subject the shot is a wide view of `city`, or of the country.
pub(super) fn of_beat(
    subject: Option<Subject>,
    beat: &Beat,
    city: Option<CityId>,
    atlas: &dyn Atlas,
    config: &DirectorConfig,
) -> Shot {
    let event = &beat.event;
    let near = if event.drama >= config.close_drama { Zoom::Close } else { Zoom::Street };
    let mut shot = Shot {
        seq: 0,
        frame: 0,
        clock: event.clock,
        zone: event.zone,
        city: None,
        pos: event.pos,
        focus: Focus::None,
        zoom: near,
        caption: beat.line.clone(),
        breaking: false,
        event: Some(event.id),
        kind: Some(event.kind.clone()),
        drama: event.drama,
        story: beat.story,
        label: None,
        headline: None,
    };
    match subject {
        Some(Subject::Resident(id)) => {
            // The resident can be somewhere else than the event by now.
            if let Some((zone, pos)) = atlas.resident_place(id) {
                shot.zone = Some(zone);
                shot.pos = Some(pos);
            }
            shot.focus = Focus::Resident { id, name: atlas.resident_name(id) };
        }
        Some(Subject::Place(id)) => {
            if let Some(site) = atlas.site(id) {
                shot.zone = Some(site.zone);
                shot.pos = Some(site.centre);
                shot.focus =
                    Focus::Place { site: Some(id), name: Some(site.name), pos: site.centre };
            } else if let Some(pos) = event.pos {
                shot.focus = Focus::Place { site: Some(id), name: None, pos };
            }
        }
        Some(Subject::Zone(zone)) => {
            shot.zone = Some(zone);
            shot.zoom = Zoom::Street;
            if let Some(pos) = event.pos {
                shot.focus = Focus::Place { site: None, name: None, pos };
            }
        }
        None => {
            // News without a place: the city that the event names, or the
            // country.
            let cities = atlas.cities();
            match city.and_then(|id| cities.iter().find(|c| c.id == id)) {
                Some(city) => {
                    shot.city = Some(city.id);
                    shot.pos = Some(city.centre);
                    shot.zoom = Zoom::City;
                }
                None => shot.zoom = Zoom::Country,
            }
            return shot;
        }
    }
    shot.city = shot.zone.and_then(|zone| atlas.zone(zone)).map(|(_, city)| city);
    shot
}

/// Writes a number of residents: `1 resident`, `212 residents`.
fn residents(count: u64) -> String {
    if count == 1 { "1 resident".to_string() } else { format!("{count} residents") }
}

/// Makes a wide view. The views take turns: the country, then each city,
/// the city with the most residents first. A country of one city shows
/// only that city.
pub(super) fn wide(turn: u64, atlas: &dyn Atlas) -> Shot {
    let mut cities = atlas.cities();
    cities.sort_by(|a, b| b.residents.cmp(&a.residents).then(a.id.cmp(&b.id)));
    let total: u64 = cities.iter().map(|city| city.residents).sum();
    let mut shot = Shot {
        seq: 0,
        frame: 0,
        clock: unsup_protocol::Clock::default(),
        zone: None,
        city: None,
        pos: None,
        focus: Focus::None,
        zoom: Zoom::Country,
        caption: format!("The whole country · {}", residents(total)),
        breaking: false,
        event: None,
        kind: None,
        drama: 0,
        story: None,
        label: None,
        headline: None,
    };
    // Turn 0 is the country when there is more than one city.
    let views = if cities.len() > 1 { cities.len() as u64 + 1 } else { cities.len() as u64 };
    if views == 0 {
        return shot;
    }
    let at = turn % views;
    let city = if cities.len() > 1 {
        match at.checked_sub(1) {
            Some(at) => cities.get(usize::try_from(at).unwrap_or(usize::MAX)),
            None => None,
        }
    } else {
        cities.first()
    };
    if let Some(city) = city {
        shot.city = Some(city.id);
        shot.pos = Some(city.centre);
        shot.zoom = Zoom::City;
        shot.caption = format!("{} · {}", city.name, residents(city.residents));
    }
    shot
}
