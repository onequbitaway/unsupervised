// What a resident does, as one of a few groups that the viewer can draw.
// The verbs are those of docs/design/game-design.md, section 5.

export type ActivityGroup =
  | 'idle'
  | 'move'
  | 'work'
  | 'rest'
  | 'eat'
  | 'trade'
  | 'talk'
  | 'crime'
  | 'law'
  | 'civic'
  | 'other';

/** Every group, in the order of the marker textures. */
export const ACTIVITY_GROUPS: readonly ActivityGroup[] = [
  'idle',
  'move',
  'work',
  'rest',
  'eat',
  'trade',
  'talk',
  'crime',
  'law',
  'civic',
  'other',
];

const GROUPS: Record<Exclude<ActivityGroup, 'idle' | 'other'>, readonly string[]> = {
  move: ['move_to', 'travel', 'walk', 'patrol'],
  work: ['work', 'gather', 'produce', 'train', 'apply_job', 'post_job', 'stock', 'unstock', 'heal'],
  rest: ['sleep', 'rest', 'wait'],
  eat: ['eat', 'use_item'],
  trade: [
    'buy', 'sell', 'set_price', 'order_place', 'order_cancel', 'market_claim', 'buy_property',
    'sell_property', 'rent', 'rent_out', 'open_business', 'close_business', 'deposit', 'withdraw',
    'loan_request', 'loan_repay', 'issue_shares', 'buy_shares', 'sell_shares', 'pay_dividend',
    'open_account', 'pick_up', 'drop', 'give', 'fence',
  ],
  talk: ['say', 'message', 'post', 'org_say', 'relate', 'propose', 'accept_proposal', 'campaign'],
  crime: ['pickpocket', 'burgle', 'rob', 'defraud', 'assault', 'vandalize', 'bribe'],
  law: ['investigate', 'search', 'arrest', 'charge', 'release', 'rule_case', 'jury_vote', 'pay_fine'],
  civic: [
    'vote', 'vote_law', 'propose_law', 'sign_law', 'veto_law', 'register_candidate', 'donate',
    'start_recall', 'sign_recall', 'appoint', 'dismiss', 'org_create', 'org_join', 'contract_sign',
    'contract_offer', 'mission_accept', 'mission_claim',
  ],
};

const BY_VERB = new Map<string, ActivityGroup>();
for (const [group, verbs] of Object.entries(GROUPS)) {
  for (const verb of verbs) BY_VERB.set(verb, group as ActivityGroup);
}

/** Returns the group of a verb. No verb is `idle`; an unknown verb is `other`. */
export function activityGroup(verb: string | null | undefined): ActivityGroup {
  if (verb === null || verb === undefined || verb === '') return 'idle';
  return BY_VERB.get(verb) ?? 'other';
}

/** Returns a verb as words for the screen: `move_to` gives `move to`. */
export function activityLabel(verb: string | null | undefined): string {
  if (verb === null || verb === undefined || verb === '') return 'idle';
  return verb.replace(/_/g, ' ');
}
