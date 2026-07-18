use protocol::{ActionType, DamageDetails, DamageModifierKind};
use serde::{Deserialize, Serialize};

use crate::constants::CharacterType;

use super::AdjustedDamageInstance;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DamageStatusContribution {
    pub status_name: String,
    pub kind: DamageModifierKind,
    pub category: i32,
    /// Average contribution across every captured hit, including hits where
    /// this status was not active.
    pub average_value: f32,
    /// Number of captured hits where this status was active.
    pub active_hits: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AverageDamageDetails {
    pub hits: u32,
    pub total_damage: u64,
    pub elemental_multiplier: f32,
    pub amplify_multiplier: f32,
    pub defense_multiplier: f32,
    pub attack_multiplier: f32,
    pub supplementary_multiplier: f32,
    pub formula_multiplier: f32,
    /// Multiplier observed from the game's final damage result, normalized by
    /// min(uncapped damage, damage cap), then including the separately emitted
    /// supplementary-damage ratio so it is comparable with the a-e formula.
    pub observed_multiplier: f32,
    pub attack_rate: f32,
    pub uncapped_damage: f32,
    pub damage_cap: f32,
    pub damage_limit_multiplier: f32,
    pub statuses: Vec<DamageStatusContribution>,
}

impl AverageDamageDetails {
    fn new(details: &DamageDetails, damage: u64) -> Self {
        Self {
            hits: 1,
            total_damage: damage,
            elemental_multiplier: details.elemental_multiplier,
            amplify_multiplier: details.amplify_multiplier,
            defense_multiplier: details.defense_multiplier,
            attack_multiplier: details.attack_multiplier,
            supplementary_multiplier: details.supplementary_multiplier,
            formula_multiplier: details.formula_multiplier,
            observed_multiplier: observed_multiplier(details, damage),
            attack_rate: details.attack_rate,
            uncapped_damage: details.uncapped_damage,
            damage_cap: details.damage_cap as f32,
            damage_limit_multiplier: details.damage_limit_multiplier,
            statuses: grouped_statuses(details)
                .into_iter()
                .map(
                    |((status_name, kind, category), value)| DamageStatusContribution {
                        status_name,
                        kind,
                        category,
                        average_value: value,
                        active_hits: 1,
                    },
                )
                .collect(),
        }
    }

    fn update(&mut self, details: &DamageDetails, damage: u64) {
        self.hits += 1;
        self.total_damage += damage;
        let hits = self.hits as f32;

        update_average(
            &mut self.elemental_multiplier,
            details.elemental_multiplier,
            hits,
        );
        update_average(
            &mut self.amplify_multiplier,
            details.amplify_multiplier,
            hits,
        );
        update_average(
            &mut self.defense_multiplier,
            details.defense_multiplier,
            hits,
        );
        update_average(&mut self.attack_multiplier, details.attack_multiplier, hits);
        update_average(
            &mut self.supplementary_multiplier,
            details.supplementary_multiplier,
            hits,
        );
        update_average(
            &mut self.formula_multiplier,
            details.formula_multiplier,
            hits,
        );
        update_average(
            &mut self.observed_multiplier,
            observed_multiplier(details, damage),
            hits,
        );
        update_average(&mut self.attack_rate, details.attack_rate, hits);
        update_average(&mut self.uncapped_damage, details.uncapped_damage, hits);
        update_average(&mut self.damage_cap, details.damage_cap as f32, hits);
        update_average(
            &mut self.damage_limit_multiplier,
            details.damage_limit_multiplier,
            hits,
        );

        // First age every existing status average for the new hit. The current
        // hit contributes zero unless the status is added below.
        let previous_weight = (hits - 1.0) / hits;
        for status in &mut self.statuses {
            status.average_value *= previous_weight;
        }

        for ((status_name, kind, category), value) in grouped_statuses(details) {
            if let Some(status) = self.statuses.iter_mut().find(|status| {
                status.status_name == status_name
                    && status.kind == kind
                    && status.category == category
            }) {
                status.average_value += value / hits;
                status.active_hits += 1;
            } else {
                self.statuses.push(DamageStatusContribution {
                    status_name,
                    kind,
                    category,
                    average_value: value / hits,
                    active_hits: 1,
                });
            }
        }
    }
}

fn update_average(average: &mut f32, value: f32, hits: f32) {
    *average += (value - *average) / hits;
}

fn observed_multiplier(details: &DamageDetails, damage: u64) -> f32 {
    let capped_base = if details.damage_cap > 0 {
        details.uncapped_damage.min(details.damage_cap as f32)
    } else {
        details.uncapped_damage
    };

    if !capped_base.is_finite() || capped_base <= 0.0 {
        return 0.0;
    }

    damage as f32 / capped_base * details.supplementary_multiplier
}

fn grouped_statuses(details: &DamageDetails) -> Vec<((String, DamageModifierKind, i32), f32)> {
    let mut statuses: Vec<((String, DamageModifierKind, i32), f32)> = Vec::new();

    for status in &details.statuses {
        if let Some((_, value)) = statuses.iter_mut().find(|((name, kind, category), _)| {
            name == &status.status_name && *kind == status.kind && *category == status.category
        }) {
            *value += status.value;
        } else {
            statuses.push((
                (status.status_name.clone(), status.kind, status.category),
                status.value,
            ));
        }
    }

    statuses
}

/// Derived stat breakdown of a particular skill
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillState {
    /// Type of action ID that this skill is
    pub action_type: ActionType,
    /// Child character this skill belongs to (pet, Id's dragonform, etc.)
    pub child_character_type: CharacterType,
    /// Number of hits this skill has done
    pub hits: u32,
    /// Minimum damage done by this skill
    pub min_damage: Option<u64>,
    /// Maximum damage done by this skill
    pub max_damage: Option<u64>,
    /// Total damage done by this skill
    pub total_damage: u64,
    /// Maximum stun value done by this skill
    pub max_stun_value: f64,
    /// Total stun value done by this skill
    pub total_stun_value: f64,
    /// Damage modifiers averaged across captured hits for this skill.
    #[serde(default)]
    pub damage_details: Option<AverageDamageDetails>,
}

impl SkillState {
    pub fn new(action_type: ActionType, child_character_type: CharacterType) -> Self {
        Self {
            action_type,
            child_character_type,
            hits: 0,
            min_damage: None,
            max_damage: None,
            total_damage: 0,
            max_stun_value: 0.0,
            total_stun_value: 0.0,
            damage_details: None,
        }
    }

    pub fn update_from_damage_event(&mut self, damage_instance: &AdjustedDamageInstance) {
        self.hits += 1;
        self.total_damage += damage_instance.event.damage as u64;
        self.max_stun_value = self.max_stun_value.max(damage_instance.stun_damage);
        self.total_stun_value += damage_instance.stun_damage;

        // Supplementary damage is emitted as a separate event, but its ratio is
        // already represented by `e` on the originating hit. Do not create a
        // second, misleading copy of the same detail record on pursuit rows.
        if !matches!(self.action_type, ActionType::SupplementaryDamage(_)) {
            if let Some(details) = &damage_instance.event.details {
                let damage = damage_instance.event.damage as u64;
                if let Some(average) = &mut self.damage_details {
                    average.update(details, damage);
                } else {
                    self.damage_details = Some(AverageDamageDetails::new(details, damage));
                }
            }
        }

        if let Some(min_damage) = self.min_damage {
            self.min_damage = Some(min_damage.min(damage_instance.event.damage as u64));
        } else {
            self.min_damage = Some(damage_instance.event.damage as u64);
        }

        if let Some(max_damage) = self.max_damage {
            self.max_damage = Some(max_damage.max(damage_instance.event.damage as u64));
        } else {
            self.max_damage = Some(damage_instance.event.damage as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use protocol::{Actor, DamageEvent, DamageStatusContribution as RawDamageStatusContribution};

    use super::*;

    #[test]
    fn updating_from_damage_event() {
        let mut skill_state = SkillState::new(ActionType::Normal(1), CharacterType::Pl0000);

        let damage_event = DamageEvent {
            source: Actor {
                index: 0,
                actor_type: 0,
                parent_actor_type: 0,
                parent_index: 0,
            },
            target: Actor {
                index: 0,
                actor_type: 0,
                parent_actor_type: 0,
                parent_index: 0,
            },
            action_id: ActionType::Normal(1),
            damage: 100,
            flags: 0,
            attack_rate: None,
            stun_value: None,
            damage_cap: None,
            details: None,
        };

        let damage_event_two = DamageEvent {
            source: Actor {
                index: 0,
                actor_type: 0,
                parent_actor_type: 0,
                parent_index: 0,
            },
            target: Actor {
                index: 0,
                actor_type: 0,
                parent_actor_type: 0,
                parent_index: 0,
            },
            action_id: ActionType::Normal(1),
            damage: 1999,
            flags: 0,
            attack_rate: None,
            stun_value: None,
            damage_cap: None,
            details: None,
        };

        skill_state.update_from_damage_event(&AdjustedDamageInstance::from_damage_event(
            &damage_event,
            None,
        ));
        skill_state.update_from_damage_event(&AdjustedDamageInstance::from_damage_event(
            &damage_event_two,
            None,
        ));

        assert_eq!(skill_state.hits, 2);
        assert_eq!(skill_state.min_damage, Some(100));
        assert_eq!(skill_state.max_damage, Some(1999));
        assert_eq!(skill_state.total_damage, 2099);
    }

    #[test]
    fn damage_details_are_averaged_by_hit() {
        let first = DamageDetails {
            elemental_multiplier: 1.2,
            amplify_multiplier: 1.0,
            defense_multiplier: 1.0,
            attack_multiplier: 1.0,
            supplementary_multiplier: 1.4,
            formula_multiplier: 1.68,
            attack_rate: 1.0,
            uncapped_damage: 270_621.0,
            damage_cap: 230_591,
            damage_limit_multiplier: 1.0,
            statuses: vec![
                RawDamageStatusContribution {
                    status_name: "StatusAttackBuff".to_string(),
                    kind: DamageModifierKind::Attack,
                    category: 0,
                    value: 0.2,
                },
                RawDamageStatusContribution {
                    status_name: "StatusAttackBuff".to_string(),
                    kind: DamageModifierKind::Attack,
                    category: 0,
                    value: 0.1,
                },
            ],
        };
        let second = DamageDetails {
            elemental_multiplier: 1.2,
            amplify_multiplier: 1.15,
            defense_multiplier: 1.15,
            attack_multiplier: 1.2,
            supplementary_multiplier: 1.8,
            formula_multiplier: 2.619,
            attack_rate: 3.0,
            uncapped_damage: 400_000.0,
            damage_cap: 300_000,
            damage_limit_multiplier: 1.3,
            statuses: vec![RawDamageStatusContribution {
                status_name: "StatusDamageLimitBuff".to_string(),
                kind: DamageModifierKind::DamageLimit,
                category: 0,
                value: 0.3,
            }],
        };

        let mut average = AverageDamageDetails::new(&first, 329_744);
        average.update(&second, 510_000);

        assert_eq!(average.hits, 2);
        assert_eq!(average.total_damage, 839_744);
        assert!((average.amplify_multiplier - 1.075).abs() < 0.0001);
        assert!((average.supplementary_multiplier - 1.6).abs() < 0.0001);
        assert!((average.formula_multiplier - 2.1495).abs() < 0.0001);
        assert!((average.attack_rate - 2.0).abs() < 0.0001);
        assert!((average.uncapped_damage - 335_310.5).abs() < 0.1);
        assert!((average.damage_cap - 265_295.5).abs() < 0.1);

        let attack = average
            .statuses
            .iter()
            .find(|status| status.kind == DamageModifierKind::Attack)
            .unwrap();
        assert_eq!(attack.active_hits, 1);
        assert!((attack.average_value - 0.15).abs() < 0.0001);

        let damage_limit = average
            .statuses
            .iter()
            .find(|status| status.kind == DamageModifierKind::DamageLimit)
            .unwrap();
        assert_eq!(damage_limit.active_hits, 1);
        assert!((damage_limit.average_value - 0.15).abs() < 0.0001);
    }
}
