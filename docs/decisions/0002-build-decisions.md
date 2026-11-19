# 0002: Decisions taken during the build

Date: 2026-11-18. Status: accepted.

These decisions came up while the parts were built. Each has a short reason.

## Kernel and runtime

1. **A finance module beside the economy module.** Bank, loans, exchange and
   shares are a topic of their own, and two smaller modules are easier to
   change than one large one.
2. **Shared family data lives in the kernel (`Kin`).** The life module reads
   the heir and the society module writes the spouse. Neither imports the
   other.
3. **A supply cap of 2^48 for each realm.** With the cap, the sum of all
   money stays far below the limit of the integer type, so a transfer can
   never fail for lack of room. Code that relied on that now panics on a
   broken invariant instead of losing value.
4. **Inbox entries are packed bytes, not value trees.** A full inbox was
   the largest part of the memory of a resident.
5. **Observations are built in batches on all cores.** An observation
   changes nothing, so the requests of one mailbox batch can run in
   parallel, grouped by zone.
6. **Sight crosses a zone border with a copy that is one frame old.** Zones
   stay independent inside a frame, and the result stays deterministic.
7. **No rollback of a single zone.** Money moves between zones, so only the
   whole world can go back to a snapshot.
8. **One kernel world for each realm in a cluster.** One router then orders
   every message, which gives the same state as one process.
9. **The cluster needs a shared secret off the loopback address.** The
   transport has no encryption and is for a private network.

## Game rules

10. **Police and judges are roles of the city government.** The society
    module has no call to make an agency without a founder. A separate
    police organization can come later.
11. **Evidence only points at the real perpetrator.** The engine never
    convicts an innocent resident. A judge or a jury can still acquit a
    guilty one.
12. **A weak city watch from the first day.** A world where a crime has no
    enforced result collapses. Agent police are much more effective than
    the watch.
13. **No `defraud` verb.** Fraud by a broken promise needs no verb.
14. **Loans need full collateral by default.** Unsecured credit let a
    throwaway resident take money and default. A pledge on property is the
    next step.
15. **Public works pay 2,800 cents for a shift.** A newcomer with no skill
    can then buy food and has a small margin.
16. **Newcomers get default routines.** A resident whose owner is away eats
    carried food and sleeps when tired.
17. **A sponsor mission creates new money through a named faucet with a
    daily limit.** A mission of a resident locks the resident's own money.
18. **A season ends with a soft reset.** Reputations keep half and cash
    above a floor is cut by half. The world and the skills stay.

## Services

19. **Moderation is at the entrance.** Every text passes the filter before
    it enters the simulation, so the simulation and the stream hold only
    moderated text.
20. **A stream delay is available and is zero by default.** The live view
    shows hidden information, for example who the thief is. A delay
    protects a competitive season.
21. **MCP is written in the gateway and speaks versions 2026-07-28,
    2025-11-25 and 2025-06-18.**
22. **Reports have a sequence number.** A client can drop a repeat and can
    order a report against a notice.
23. **An agent follows the action ids of its own plan.** An agent that
    looked only at the activity of its resident took a routine action for
    its own and lost the resident.
