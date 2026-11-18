//! Ids of everything that the life module defines: parameters, attributes,
//! conditions and sinks. They are resolved once, when the plugin is built.

use unsup_kernel::{AttrDef, AttrId, AttrShow, Builder, CondDef, CondId, ParamId, Realm, SinkId};

use crate::names::{SKILLS, attr, cond};

/// Declares the struct [`Params`] with one `ParamId` for each line.
macro_rules! params {
    ($($field:ident => $name:literal,)*) => {
        /// Ids of the parameters of `data/balance.toml` that the code reads
        /// by name.
        #[derive(Clone, Copy, Debug)]
        pub(crate) struct Params {
            $(pub(crate) $field: ParamId,)*
        }

        impl Params {
            fn new(b: &Builder) -> Params {
                Params { $($field: b.param_id($name),)* }
            }
        }
    };
}

params! {
    walk_ticks_road => "life.walk_ticks_road",
    walk_ticks_pavement => "life.walk_ticks_pavement",
    walk_ticks_floor => "life.walk_ticks_floor",
    walk_ticks_grass => "life.walk_ticks_grass",
    walk_ticks_field => "life.walk_ticks_field",
    walk_ticks_trees => "life.walk_ticks_trees",
    path_search_max => "life.path_search_max",
    walk_timeout => "life.walk_timeout",
    travel_fare => "life.travel_fare",
    travel_ticks => "life.travel_ticks",
    food_decay => "life.food_decay_per_hour",
    energy_decay => "life.energy_decay_per_hour",
    mood_decay => "life.mood_decay_per_hour",
    need_low => "life.need_low",
    low_need_mood => "life.low_need_mood_per_hour",
    starve_health => "life.starve_health_per_hour",
    exhaust_health => "life.exhaust_health_per_hour",
    health_regen => "life.health_regen_per_hour",
    eat_ticks => "life.eat_ticks",
    meal_mood => "life.meal_mood",
    use_item_ticks => "life.use_item_ticks",
    sleep_min_ticks => "life.sleep_min_ticks",
    sleep_max_ticks => "life.sleep_max_ticks",
    sleep_energy_good => "life.sleep_energy_good_per_hour",
    sleep_energy_fair => "life.sleep_energy_fair_per_hour",
    sleep_energy_poor => "life.sleep_energy_poor_per_hour",
    sleep_mood_good => "life.sleep_mood_good_per_hour",
    sleep_mood_fair => "life.sleep_mood_fair_per_hour",
    sleep_mood_poor => "life.sleep_mood_poor_per_hour",
    rest_max_ticks => "life.rest_max_ticks",
    rest_energy => "life.rest_energy_per_hour",
    rest_mood => "life.rest_mood_per_hour",
    rest_mood_park => "life.rest_mood_park_per_hour",
    heal_fee => "life.heal_fee_per_point",
    heal_rate => "life.heal_health_per_hour",
    train_min_ticks => "life.train_min_ticks",
    train_max_ticks => "life.train_max_ticks",
    train_fee => "life.train_fee_per_hour",
    train_xp => "life.train_xp_per_hour",
    learn_min_pct => "life.learn_min_pct",
    spawn_age => "life.spawn_age",
    old_age_from => "life.old_age_from",
    old_age_death_bp => "life.old_age_death_bp",
    old_age_death_rise_bp => "life.old_age_death_rise_bp",
    asleep_sight => "life.asleep_sight_radius",
    hurt_below => "life.inspect_hurt_below",
    give_range => "life.give_range",
    give_ticks => "life.give_ticks",
    carry_ticks => "life.carry_ticks",
    reflexes => "life.reflexes",
    reflex_eat_below => "life.reflex_eat_below",
    reflex_doze_below => "life.reflex_doze_below",
    reflex_doze_energy => "life.reflex_doze_energy",
    spawn_routines => "life.spawn_routines",
    routine_eat_below => "life.routine_eat_below",
    routine_sleep_below => "life.routine_sleep_below",
    drama_meal => "life.drama.meal",
    drama_slept => "life.drama.slept",
    drama_item_used => "life.drama.item_used",
    drama_healed => "life.drama.healed",
    drama_trained => "life.drama.trained",
    drama_level => "life.drama.level_gained",
    drama_gift => "life.drama.gift",
    drama_traveled => "life.drama.traveled",
    drama_will => "life.drama.will",
    drama_birthday => "life.drama.birthday",
    sight_radius => "kernel.sight_radius",
}

/// Ids of the attributes, conditions, sinks and parameters of the module.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Handles {
    pub(crate) food: AttrId,
    pub(crate) energy: AttrId,
    pub(crate) health: AttrId,
    pub(crate) mood: AttrId,
    pub(crate) age: AttrId,
    /// One attribute for each entry of [`SKILLS`], in the same order.
    pub(crate) skills: [AttrId; SKILLS.len()],
    pub(crate) asleep: CondId,
    pub(crate) hospitalized: CondId,
    /// Sink of food that was eaten.
    pub(crate) eaten: SinkId,
    /// Sink of items that were used.
    pub(crate) used: SinkId,
    pub(crate) p: Params,
}

impl Handles {
    /// Loads the data file and declares the attributes, the conditions and
    /// the sinks. It is safe to call it in both passes of the builder.
    pub(crate) fn register(b: &mut Builder) -> Handles {
        b.load_data(include_str!("../data/balance.toml"));

        let need = |name: &str, summary: &str| {
            AttrDef::new(name).range(0, attr::NEED_MAX).default(attr::NEED_MAX).summary(summary)
        };
        let food = b.attr(need(attr::FOOD, "Food, 0 to 1000. At 0 the resident starves."));
        let energy =
            b.attr(need(attr::ENERGY, "Energy, 0 to 1000. At 0 the resident is exhausted."));
        let health = b.attr(need(attr::HEALTH, "Health, 0 to 1000. At 0 the resident dies."));
        let mood = b.attr(need(attr::MOOD, "Mood, 0 to 1000. A low mood slows work and learning."));
        let age = b.attr(AttrDef::new(attr::AGE).range(0, 1_000).summary("Age in years."));
        let skills = SKILLS.map(|skill| {
            b.attr(
                AttrDef::new(format!("{}{skill}", attr::SKILL_PREFIX))
                    .range(0, i64::MAX)
                    .show(AttrShow::Full)
                    .summary(format!(
                        "Experience in {skill}. The level is floor(sqrt(experience / 100))."
                    )),
            )
        });

        let asleep = b.condition(
            CondDef::new(cond::ASLEEP).summary("The resident sleeps. Its sight is reduced."),
        );
        let hospitalized = b.condition(
            CondDef::new(cond::HOSPITALIZED)
                .blocking()
                .summary("The resident is in hospital until it is healed."),
        );

        Handles {
            food,
            energy,
            health,
            mood,
            age,
            skills,
            asleep,
            hospitalized,
            eaten: b.sink("consumption.food"),
            used: b.sink("consumption.use"),
            p: Params::new(b),
        }
    }
}

impl Handles {
    /// Returns the drama score of an event from its parameter.
    pub(crate) fn drama(&self, realm: &Realm, param: ParamId) -> u8 {
        u8::try_from(realm.param(param).clamp(0, 100)).unwrap_or(0)
    }

    /// Returns a parameter as a number of ticks, at least 1.
    pub(crate) fn ticks(&self, realm: &Realm, param: ParamId) -> u32 {
        u32::try_from(realm.param(param)).unwrap_or(1).max(1)
    }
}
