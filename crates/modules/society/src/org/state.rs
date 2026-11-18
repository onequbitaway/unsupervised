//! The registry of organizations. It is state of the global realm and the
//! source of truth for memberships.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unsup_kernel::{OrgId, ResidentId, Resource};

use crate::perms;
use crate::social::Meter;

/// One organization: members, roles, invitations and grants. Its treasury
/// is the money of `Holder::Org(id)` in the ledger of the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Org {
    pub(crate) id: OrgId,
    /// The name that the founder wrote. Empty for a government
    /// organization.
    pub(crate) name: String,
    pub(crate) founder: Option<ResidentId>,
    /// True when a resident can join without an invitation.
    pub(crate) open: bool,
    /// Role name to the permissions that the role lists, sorted.
    pub(crate) roles: BTreeMap<String, Vec<String>>,
    /// Member to role name.
    pub(crate) members: BTreeMap<ResidentId, String>,
    /// Invited resident to the last frame in which it can join.
    pub(crate) invites: BTreeMap<ResidentId, u64>,
    /// Power of the state to the government organizations that granted it.
    pub(crate) grants: BTreeMap<String, BTreeSet<OrgId>>,
    /// Texts that `org_say` sent to the members in the current hour.
    pub(crate) said: Meter,
}

impl Org {
    /// Returns the id.
    #[must_use]
    pub fn id(&self) -> OrgId {
        self.id
    }

    /// Returns the name that the founder wrote. It is text of a resident:
    /// wrap it in `Untrusted` before it goes to an agent. A government
    /// organization has an empty name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the resident that made the organization, if a resident did.
    #[must_use]
    pub fn founder(&self) -> Option<ResidentId> {
        self.founder
    }

    /// Returns true for the nation, the central bank and the city
    /// governments.
    #[must_use]
    pub fn is_government(&self) -> bool {
        perms::is_government(self.id)
    }

    /// Returns the members with the names of their roles, ordered by
    /// resident.
    pub fn members(&self) -> impl Iterator<Item = (ResidentId, &str)> + '_ {
        self.members.iter().map(|(&resident, role)| (resident, role.as_str()))
    }

    /// Returns the number of members.
    #[must_use]
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Returns the number of invitations in the record. An invitation that
    /// ended stays in the record until the clean-up that runs each game
    /// hour.
    #[must_use]
    pub fn invite_count(&self) -> usize {
        self.invites.len()
    }

    /// Returns the role of a member.
    #[must_use]
    pub fn role_of(&self, resident: ResidentId) -> Option<&str> {
        self.members.get(&resident).map(String::as_str)
    }

    /// Returns the members with a role, ordered by resident.
    #[must_use]
    pub fn holders(&self, role: &str) -> Vec<ResidentId> {
        self.members.iter().filter(|(_, r)| r.as_str() == role).map(|(&id, _)| id).collect()
    }

    /// Returns the permissions that a role lists. Some of them can be
    /// without effect; see [`Org::effective`].
    #[must_use]
    pub fn role_permissions(&self, role: &str) -> &[String] {
        self.roles.get(role).map_or(&[], Vec::as_slice)
    }

    /// Returns the government organizations by whose power this
    /// organization holds a `law.*`, `gov.*` or `bank.*` permission: itself
    /// when it is a government that has the power, and every grantor.
    #[must_use]
    pub fn sources(&self, perm: &str) -> Vec<OrgId> {
        let mut sources = Vec::new();
        if perms::holds_by_itself(self.id, perm) {
            sources.push(self.id);
        }
        if let Some(grantors) = self.grants.get(perm) {
            sources.extend(grantors.iter().copied().filter(|g| *g != self.id));
        }
        sources
    }

    /// Returns true when a permission has an effect in this organization:
    /// always for an ordinary permission, and for a power of the state only
    /// while the organization holds it.
    #[must_use]
    pub fn holds(&self, perm: &str) -> bool {
        !perms::is_sovereign(perm) || !self.sources(perm).is_empty()
    }

    /// Returns the permissions of a role that have an effect, as the zones
    /// see them. A power of the state is there once by its name and once
    /// for each source as `<permission>@<organization>`.
    #[must_use]
    pub fn effective(&self, role: &str) -> Vec<String> {
        let mut out = Vec::new();
        for perm in self.role_permissions(role) {
            if !perms::is_sovereign(perm) {
                out.push(perm.clone());
                continue;
            }
            let sources = self.sources(perm);
            if !sources.is_empty() {
                out.push(perm.clone());
                out.extend(sources.into_iter().map(|source| perms::scoped(perm, source)));
            }
        }
        out
    }

    /// Returns true when a resident is a member with a role that gives the
    /// permission, and the permission has an effect.
    #[must_use]
    pub fn can(&self, resident: ResidentId, perm: &str) -> bool {
        self.role_of(resident).is_some_and(|role| {
            self.role_permissions(role).iter().any(|p| p == perm) && self.holds(perm)
        })
    }

    /// Returns the number of members that can manage the organization.
    pub(crate) fn managers(&self) -> usize {
        self.members.keys().filter(|&&m| self.can(m, perms::ORG_MANAGE)).count()
    }

    /// Returns true when a resident has an invitation that is open in
    /// `frame`.
    pub(crate) fn invited(&self, resident: ResidentId, frame: u64) -> bool {
        self.invites.get(&resident).is_some_and(|&expires| expires >= frame)
    }

    /// Returns the number of invitations that are open in `frame`.
    pub(crate) fn open_invites(&self, frame: u64) -> usize {
        self.invites.values().filter(|&&expires| expires >= frame).count()
    }
}

/// Every organization. A resource of the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Orgs {
    orgs: BTreeMap<OrgId, Org>,
    /// Lower-case name to organization, so that a name is used once.
    names: BTreeMap<String, OrgId>,
    by_member: BTreeMap<ResidentId, BTreeSet<OrgId>>,
    invited: BTreeMap<ResidentId, BTreeSet<OrgId>>,
}

impl Resource for Orgs {
    const NAME: &'static str = "society.orgs";
}

/// Returns the form of a name that must be unique.
fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

impl Orgs {
    /// Returns one organization.
    #[must_use]
    pub fn get(&self, org: OrgId) -> Option<&Org> {
        self.orgs.get(&org)
    }

    /// Returns the number of organizations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.orgs.len()
    }

    /// Returns true when there is no organization.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orgs.is_empty()
    }

    /// Returns the organizations of a resident, ordered by id.
    pub fn of_member(&self, resident: ResidentId) -> impl Iterator<Item = &Org> + '_ {
        self.by_member.get(&resident).into_iter().flatten().filter_map(|org| self.orgs.get(org))
    }

    /// Returns the number of organizations of a resident.
    #[must_use]
    pub fn membership_count(&self, resident: ResidentId) -> usize {
        self.by_member.get(&resident).map_or(0, BTreeSet::len)
    }

    /// Returns the organizations with an invitation for a resident that is
    /// open in `frame`, ordered by id, each with the last frame in which
    /// the resident can join.
    pub(crate) fn invitations(
        &self,
        resident: ResidentId,
        frame: u64,
    ) -> impl Iterator<Item = (&Org, u64)> + '_ {
        self.invited
            .get(&resident)
            .into_iter()
            .flatten()
            .filter_map(|org| self.orgs.get(org))
            .filter_map(move |org| org.invites.get(&resident).map(|&expires| (org, expires)))
            .filter(move |&(_, expires)| expires >= frame)
    }

    pub(crate) fn get_mut(&mut self, org: OrgId) -> Option<&mut Org> {
        self.orgs.get_mut(&org)
    }

    /// Returns true when an organization has this name. Case and outer
    /// spaces do not count.
    pub(crate) fn name_taken(&self, name: &str) -> bool {
        self.names.contains_key(&name_key(name))
    }

    /// Adds an organization.
    pub(crate) fn insert(&mut self, org: Org) {
        if !org.name.is_empty() {
            self.names.insert(name_key(&org.name), org.id);
        }
        self.orgs.insert(org.id, org);
    }

    /// Removes an organization that has no member, with its invitations.
    pub(crate) fn remove(&mut self, id: OrgId) -> Option<Org> {
        let org = self.orgs.remove(&id)?;
        self.names.remove(&name_key(&org.name));
        for resident in org.invites.keys() {
            unlink(&mut self.invited, *resident, id);
        }
        for resident in org.members.keys() {
            unlink(&mut self.by_member, *resident, id);
        }
        Some(org)
    }

    /// Makes a resident a member with a role, or changes its role. An open
    /// invitation of the resident ends.
    pub(crate) fn set_member(&mut self, id: OrgId, resident: ResidentId, role: &str) {
        let Some(org) = self.orgs.get_mut(&id) else {
            return;
        };
        org.members.insert(resident, role.to_string());
        self.by_member.entry(resident).or_default().insert(id);
        self.remove_invite(id, resident);
    }

    /// Ends a membership. Returns the role that the member had.
    pub(crate) fn remove_member(&mut self, id: OrgId, resident: ResidentId) -> Option<String> {
        let role = self.orgs.get_mut(&id)?.members.remove(&resident)?;
        unlink(&mut self.by_member, resident, id);
        Some(role)
    }

    /// Records an invitation that is open until the frame `expires`. It
    /// replaces an older invitation of the same resident.
    pub(crate) fn add_invite(&mut self, id: OrgId, resident: ResidentId, expires: u64) {
        if let Some(org) = self.orgs.get_mut(&id) {
            org.invites.insert(resident, expires);
            self.invited.entry(resident).or_default().insert(id);
        }
    }

    /// Forgets every invitation that is no longer open in `frame`.
    pub(crate) fn expire_invites(&mut self, frame: u64) {
        for org in self.orgs.values_mut() {
            let id = org.id;
            let invited = &mut self.invited;
            org.invites.retain(|&resident, expires| {
                let open = *expires >= frame;
                if !open {
                    unlink(invited, resident, id);
                }
                open
            });
        }
    }

    pub(crate) fn remove_invite(&mut self, id: OrgId, resident: ResidentId) {
        if let Some(org) = self.orgs.get_mut(&id) {
            org.invites.remove(&resident);
        }
        unlink(&mut self.invited, resident, id);
    }

    /// Returns the ids of the organizations of a resident and of those that
    /// invited it. The caller removes the resident from each.
    pub(crate) fn links_of(&self, resident: ResidentId) -> (Vec<OrgId>, Vec<OrgId>) {
        let list = |map: &BTreeMap<ResidentId, BTreeSet<OrgId>>| {
            map.get(&resident).map(|set| set.iter().copied().collect()).unwrap_or_default()
        };
        (list(&self.by_member), list(&self.invited))
    }
}

/// Removes one organization from the set of a resident in an index.
fn unlink(index: &mut BTreeMap<ResidentId, BTreeSet<OrgId>>, resident: ResidentId, org: OrgId) {
    if let Some(set) = index.get_mut(&resident) {
        set.remove(&org);
        if set.is_empty() {
            index.remove(&resident);
        }
    }
}
