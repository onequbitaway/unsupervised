// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { panelFacts } from '../state/profile';
import { addItems, advance, emptyTicker } from '../state/ticker';
import type { Profile } from '../state/types';
import { lifePage } from '../state/life';
import { lowerThird } from '../state/lowerthird';
import { BreakingBanner, Curtain, LowerThirdBar, SidePanel } from './broadcast';
import { ClipBar } from './clipbar';
import { Controls } from './controls';
import { button, el, setShown, setText } from './dom';
import { Hud } from './hud';
import { keyAction, wheelFactor } from './input';
import { cssColor, Legend } from './legend';
import { LifePageView } from './lifepage';
import { ResidentPanel } from './panel';
import { tagOf, TickerBar } from './tickerbar';

/** Text that tries to become markup. */
const HOSTILE = '<img src=x onerror="window.hacked=1"><script>window.hacked=1</script><b>bold</b>';

/** Returns the number of elements that text from the world must never make. */
function injected(root: Element): number {
  return root.querySelectorAll('img, script, b, iframe, svg').length;
}

describe('dom helpers', () => {
  it('puts text into the page as text, never as markup', () => {
    const node = el('p', 'note', HOSTILE);
    expect(node.className).toBe('note');
    expect(node.textContent).toBe(HOSTILE);
    expect(node.children).toHaveLength(0);
    setText(node, `${HOSTILE}!`);
    expect(node.textContent).toBe(`${HOSTILE}!`);
    expect(node.children).toHaveLength(0);
  });

  it('shows and hides an element, and makes a button that calls its handler', () => {
    const node = el('div');
    setShown(node, false);
    expect(node.hidden).toBe(true);
    setShown(node, true);
    expect(node.hidden).toBe(false);
    let presses = 0;
    const press = button('Go', 'Go there', () => {
      presses += 1;
    });
    press.click();
    expect(presses).toBe(1);
    expect(press.type).toBe('button');
    expect(press.getAttribute('aria-label')).toBe('Go there');
  });
});

describe('TickerBar', () => {
  function tickerWith(lines: { id: string; drama: number; text: string; actor?: number }[]) {
    const items = lines.map((line) => ({ at: 0, zone: null, actor: line.actor ?? null, ...line }));
    return advance(addItems(emptyTicker(), items, 0), 0);
  }

  it('is hidden without news', () => {
    const bar = new TickerBar(() => {});
    bar.update(emptyTicker());
    expect(bar.root.hidden).toBe(true);
  });

  it('shows the most dramatic line in the main line and the others in the list', () => {
    const bar = new TickerBar(() => {});
    bar.update(
      tickerWith([
        { id: '1-0', drama: 30, text: 'Ada opened a shop' },
        { id: '1-1', drama: 90, text: 'Election decided' },
        { id: '1-2', drama: 5, text: 'Bo: “hi”' },
      ]),
    );
    expect(bar.root.hidden).toBe(false);
    expect(bar.root.querySelector('.ticker-tag')?.textContent).toBe('HEADLINE');
    expect(bar.root.querySelector('.ticker-line')?.textContent).toBe('Election decided');
    // The list has notable lines only: the routine line is not there.
    const rows = [...bar.root.querySelectorAll<HTMLElement>('.ticker-row')].filter((row) => !row.hidden);
    expect(rows.map((row) => row.textContent)).toEqual(['Ada opened a shop']);
    expect(rows[0]?.dataset.tag).toBe('news');
  });

  it('shows text from residents as text', () => {
    const bar = new TickerBar(() => {});
    bar.update(tickerWith([{ id: '1-0', drama: 90, text: HOSTILE }, { id: '1-1', drama: 40, text: HOSTILE }]));
    expect(bar.root.querySelector('.ticker-line')?.textContent).toBe(HOSTILE);
    expect(bar.root.querySelector('.ticker-row')?.textContent).toBe(HOSTILE);
    expect(injected(bar.root)).toBe(0);
  });

  it('gives the pressed line to the handler', () => {
    const picked: string[] = [];
    const bar = new TickerBar((item) => picked.push(item.id));
    bar.update(tickerWith([{ id: '1-0', drama: 30, text: 'a' }, { id: '1-1', drama: 90, text: 'b' }]));
    bar.root.querySelector<HTMLElement>('.ticker-main')?.click();
    bar.root.querySelector<HTMLElement>('.ticker-row')?.click();
    expect(picked).toEqual(['1-1', '1-0']);
  });

  it('names a line by its drama score', () => {
    expect([tagOf(71), tagOf(70), tagOf(11), tagOf(10)]).toEqual(['HEADLINE', 'NEWS', 'NEWS', 'LOCAL']);
  });
});

describe('ResidentPanel', () => {
  const profile: Profile = {
    id: 7,
    name: HOSTILE,
    model: HOSTILE,
    alive: true,
    zone: 3,
    zone_name: HOSTILE,
    city: 1,
    city_name: 'Tokentown',
    pos: { x: 1, y: 1 },
    frame: 9,
    events: [
      {
        id: '3-0',
        frame: 3,
        clock: { year: 1, day: 1, hour: 2, minute: 3 },
        kind: 'society.said',
        actor: 7,
        actor_name: HOSTILE,
        drama: 3,
        data: { text: { untrusted: true, from: 7, text: HOSTILE } },
      },
    ],
    life: { jobs: [], homes: [], deals: [], crimes: [], offices: [], moments: [] },
  };

  function view(following = false) {
    return {
      facts: panelFacts({ id: 7, live: null, profile, unknown: false }),
      color: '#ff6b6b',
      following,
      link: 'http://host/?watch=7',
    };
  }

  it('shows a resident: name, facts, events and the watch link', () => {
    const panel = new ResidentPanel({ close: () => {}, follow: () => {} });
    expect(panel.root.hidden).toBe(true);
    panel.show(view());
    expect(panel.root.hidden).toBe(false);
    // A name has at most 40 characters in the panel, and 22 in a place.
    expect(panel.root.querySelector('.panel-name')?.textContent).toBe(`${HOSTILE.slice(0, 39)}…`);
    const facts = [...panel.root.querySelectorAll('dd')].map((node) => node.textContent);
    expect(facts[0]).toBe('#7');
    expect(facts[2]).toBe(`Tokentown · ${HOSTILE.slice(0, 21)}…`);
    expect(facts[3]).toBe('idle');
    expect(panel.root.querySelectorAll('.panel-events li')).toHaveLength(1);
    expect(panel.root.querySelector('.panel-events time')?.textContent).toBe('Year 1 · Day 1 · 02:03');
    expect(panel.root.querySelector('.panel-link')?.getAttribute('href')).toBe('http://host/?watch=7');
    panel.hide();
    expect(panel.root.hidden).toBe(true);
  });

  it('shows every text from residents as text', () => {
    const panel = new ResidentPanel({ close: () => {}, follow: () => {} });
    panel.show(view());
    expect(injected(panel.root)).toBe(0);
    expect(panel.root.querySelector('.panel-events span')?.textContent).toContain('<img src=x');
    expect((globalThis as { hacked?: number }).hacked).toBeUndefined();
  });

  it('tells the handlers to follow, to stop and to close', () => {
    const calls: string[] = [];
    const panel = new ResidentPanel({ close: () => calls.push('close'), follow: (on) => calls.push(`follow ${on}`) });
    panel.show(view(false));
    const follow = panel.root.querySelector<HTMLButtonElement>('.panel-follow');
    expect(follow?.textContent).toBe('Follow');
    follow?.click();
    panel.show(view(true));
    expect(follow?.textContent).toBe('Stop following');
    follow?.click();
    panel.root.querySelector<HTMLButtonElement>('.panel-close')?.click();
    expect(calls).toEqual(['follow true', 'follow false', 'close']);
  });
});

describe('Hud', () => {
  it('shows the time, the numbers and the place, and a notice only when there is one', () => {
    const hud = new Hud();
    hud.update({ clock: { year: 2, day: 14, hour: 7, minute: 5 }, residents: 1234, viewers: 1, place: HOSTILE, notice: '' });
    expect(hud.root.querySelector('.hud-clock')?.textContent).toBe('Year 2 · Day 14 · 07:05');
    expect(hud.root.querySelector('.hud-counts')?.textContent).toBe('1,234 residents · 1 watching');
    expect(hud.root.querySelector('.hud-place')?.textContent).toBe(HOSTILE);
    expect(injected(hud.root)).toBe(0);
    expect(hud.root.querySelector<HTMLElement>('.hud-notice')?.hidden).toBe(true);
    hud.update({ clock: null, residents: 1, viewers: 0, place: '', notice: 'Connecting…' });
    expect(hud.root.querySelector('.hud-counts')?.textContent).toBe('1 resident · 0 watching');
    expect(hud.root.querySelector<HTMLElement>('.hud-notice')?.hidden).toBe(false);
    expect(hud.root.querySelector('.hud-notice')?.textContent).toBe('Connecting…');
  });
});

describe('Controls', () => {
  it('calls the handlers and marks the level of the camera', () => {
    const calls: string[] = [];
    const controls = new Controls({ zoom: (factor) => calls.push(`zoom ${factor}`), level: (level) => calls.push(level) });
    const buttons = [...controls.root.querySelectorAll('button')];
    expect(buttons.map((node) => node.textContent)).toEqual(['Country', 'City', 'Street', '+', '−']);
    for (const node of buttons) node.click();
    expect(calls).toEqual(['country', 'city', 'street', 'zoom 1.25', 'zoom 0.8']);
    controls.setLevel('city');
    expect(buttons.map((node) => node.classList.contains('active'))).toEqual([false, true, false, false, false]);
    controls.setLevel('street');
    expect(buttons.map((node) => node.getAttribute('aria-pressed'))).toEqual(['false', 'false', 'true', null, null]);
  });
});

describe('Legend', () => {
  it('is shown at the country level, with the number of the fullest zone', () => {
    const legend = new Legend();
    legend.update(false, new Map([[0, 500]]));
    expect(legend.root.hidden).toBe(true);
    legend.update(true, new Map([[0, 500], [1, 20]]));
    expect(legend.root.hidden).toBe(false);
    expect(legend.root.querySelector('.legend-most')?.textContent).toBe('500');
    // A small world still has the floor as its greatest number.
    legend.update(true, new Map([[0, 3]]));
    expect(legend.root.querySelector('.legend-most')?.textContent).toBe('20');
    expect(cssColor(0x00ff8a)).toBe('#00ff8a');
  });
});

describe('keys and wheel', () => {
  it('maps the keys of the viewer and no other key', () => {
    expect(keyAction('+')).toBe('in');
    expect(keyAction('-')).toBe('out');
    expect(keyAction('1')).toBe('country');
    expect(keyAction('3')).toBe('street');
    expect(keyAction('ArrowLeft')).toBe('left');
    expect(keyAction('w')).toBe('up');
    expect(keyAction('Escape')).toBe('escape');
    expect(keyAction('x')).toBeNull();
    expect(keyAction('constructor')).toBeNull();
  });

  it('the wheel zooms in when it turns away, and never by a large step', () => {
    expect(wheelFactor(0, false)).toBe(1);
    expect(wheelFactor(-100, false)).toBeGreaterThan(1);
    expect(wheelFactor(100, false)).toBeLessThan(1);
    expect(wheelFactor(-100, false) * wheelFactor(100, false)).toBeCloseTo(1);
    expect(wheelFactor(-100_000, false)).toBe(wheelFactor(-240, false));
    // A pinch gives small numbers and zooms more for each of them.
    expect(wheelFactor(-10, true)).toBeGreaterThan(wheelFactor(-10, false));
  });
});

describe('the page of a broadcast', () => {
  const clock = { year: 1, day: 1, hour: 2, minute: 3 };
  const layout = {
    frame: 0,
    clock,
    cities: [{ id: 0, name: HOSTILE, bounds: { x0: 0, y0: 0, x1: 64, y1: 64 }, zones: [0] }],
    zones: [{ id: 0, city: 0, name: 'Downtown', bounds: { x0: 0, y0: 0, x1: 64, y1: 64 }, residents: 0 }],
  };
  const shot = {
    seq: 2,
    frame: 9,
    clock,
    zone: 0,
    focus: { type: 'resident' as const, id: 7, name: HOSTILE },
    zoom: 'close' as const,
    caption: HOSTILE,
    breaking: true,
    drama: 80,
    story: 1,
    label: HOSTILE,
    headline: `story ${HOSTILE}`,
  };

  it('shows the lower third as text and hides it without a shot', () => {
    const bar = new LowerThirdBar();
    expect(bar.root.hidden).toBe(true);
    bar.update(lowerThird(shot, layout));
    expect(bar.root.hidden).toBe(false);
    expect(bar.root.classList.contains('breaking')).toBe(true);
    expect(bar.root.querySelector('.lower-caption')?.textContent).toBe(HOSTILE);
    expect(bar.root.querySelector('.lower-headline')?.textContent).toBe(`story ${HOSTILE}`);
    expect(bar.root.querySelector('.lower-place')?.textContent).toContain('Downtown');
    expect(injected(bar.root)).toBe(0);
    // A shot whose headline repeats the caption shows no second line.
    bar.update(lowerThird({ ...shot, headline: HOSTILE, breaking: false }, layout));
    expect((bar.root.querySelector('.lower-headline') as HTMLElement).hidden).toBe(true);
    expect(bar.root.classList.contains('breaking')).toBe(false);
    bar.update(lowerThird(null, layout));
    expect(bar.root.hidden).toBe(true);
  });

  it('shows the breaking-news banner only with a line', () => {
    const banner = new BreakingBanner();
    expect(banner.root.hidden).toBe(true);
    banner.update(HOSTILE);
    expect(banner.root.hidden).toBe(false);
    expect(banner.root.querySelector('.breaking-line')?.textContent).toBe(HOSTILE);
    expect(banner.root.querySelector('.breaking-tag')?.textContent).toBe('BREAKING NEWS');
    expect(injected(banner.root)).toBe(0);
    banner.update('');
    expect(banner.root.hidden).toBe(true);
  });

  it('shows one card with its rows, and nothing without a card', () => {
    const panel = new SidePanel();
    expect(panel.root.hidden).toBe(true);
    const card = {
      id: 'board:wealth',
      title: HOSTILE,
      rows: [
        { rank: '1', label: HOSTILE, value: '$9.00' },
        { rank: '', label: 'No rank', value: '' },
      ],
    };
    panel.update(card, 1, 3);
    expect(panel.root.hidden).toBe(false);
    expect(panel.root.dataset.card).toBe('board');
    expect(panel.root.querySelector('.side-title')?.textContent).toBe(HOSTILE);
    const rows = [...panel.root.querySelectorAll('.side-row')];
    expect(rows.map((row) => row.children.length)).toEqual([3, 1]);
    expect(rows[0]?.querySelector('.side-label')?.textContent).toBe(HOSTILE);
    expect(rows[0]?.querySelector('.side-value')?.textContent).toBe('$9.00');
    // Three cards in the rotation: three dots, the second one on.
    const dots = [...panel.root.querySelectorAll('.side-dot')];
    expect(dots.map((dot) => dot.classList.contains('on'))).toEqual([false, true, false]);
    expect(injected(panel.root)).toBe(0);
    // One card alone has no dots.
    panel.update(card, 0, 1);
    expect(panel.root.querySelectorAll('.side-dot')).toHaveLength(0);
    panel.update(null, 0, 0);
    expect(panel.root.hidden).toBe(true);
  });

  it('makes the picture as dark as the cut says', () => {
    const curtain = new Curtain();
    curtain.update(0.5);
    expect(curtain.root.style.opacity).toBe('0.5');
    curtain.update(1);
    expect(curtain.root.style.opacity).toBe('1');
    curtain.update(0);
    expect(curtain.root.style.opacity).toBe('0');
  });
});

describe('LifePageView', () => {
  const clock = { year: 1, day: 1, hour: 2, minute: 3 };
  const entry = (frame: number, line: string) => ({ event: `${frame}-0`, frame, clock, kind: 'x.y', drama: 20, line });
  const profile: Profile = {
    id: 7,
    name: HOSTILE,
    model: HOSTILE,
    alive: true,
    zone: 3,
    zone_name: HOSTILE,
    city: 1,
    city_name: 'Tokentown',
    pos: { x: 1, y: 1 },
    frame: 9,
    events: [],
    life: {
      born: entry(1, 'Ada arrived in Plaza'),
      jobs: [entry(5, HOSTILE)],
      homes: [],
      deals: [],
      crimes: [entry(8, 'Bo stole $45.00 from Ada')],
      offices: [],
      moments: [],
    },
  };
  const links = { watch: 'http://host/?watch=7', live: 'http://host/', clip: (event: string) => `http://host/?clip=${event}` };

  it('shows the life story by chapter, as text, with a link to each clip', () => {
    const view = new LifePageView();
    view.update(lifePage(profile), links);
    expect(view.root.querySelector('.life-name')?.textContent).toBe(lifePage(profile).name);
    const chapters = [...view.root.querySelectorAll('.life-chapter')] as HTMLElement[];
    expect(chapters.map((chapter) => chapter.dataset.chapter)).toEqual(['born', 'jobs', 'crimes']);
    expect(chapters[1]?.querySelector('.life-text')?.textContent).toBe(HOSTILE);
    expect(chapters[2]?.querySelector('.life-replay')?.getAttribute('href')).toBe('http://host/?clip=8-0');
    expect(view.root.querySelector('.life-link.primary')?.getAttribute('href')).toBe('http://host/?watch=7');
    expect((view.root.querySelector('.life-note') as HTMLElement).hidden).toBe(true);
    expect(injected(view.root)).toBe(0);
  });

  it('says that the chronicle has nothing, and gives no watch link for a resident that left', () => {
    const view = new LifePageView();
    const empty = { jobs: [], homes: [], deals: [], crimes: [], offices: [], moments: [] };
    view.update(lifePage({ ...profile, alive: false, life: empty }), links);
    expect(view.root.querySelector('.life-note')?.textContent).toBe('The chronicle has no event of this resident yet.');
    expect(view.root.querySelector('.life-link.primary')).toBeNull();
    expect(view.root.querySelector('.life-status')?.classList.contains('gone')).toBe(true);
    view.showNote('No resident with the id 7 is known.');
    expect(view.root.querySelector('.life-note')?.textContent).toBe('No resident with the id 7 is known.');
  });
});

describe('ClipBar', () => {
  const model = {
    line: HOSTILE,
    when: 'Year 1 · Day 1 · 02:03',
    place: 'Tokentown · Downtown',
    progress: 0.25,
    playing: true,
    pin: 'open' as const,
    note: '',
  };

  it('shows the line of the clip as text, and how far the clip is', () => {
    const bar = new ClipBar({ toggle: () => {}, restart: () => {}, pin: () => {} }, 'http://host/?clip=5-0', 'http://host/');
    bar.update(model);
    expect(bar.root.querySelector('.clip-line')?.textContent).toBe(HOSTILE);
    expect((bar.root.querySelector('.clip-fill') as HTMLElement).style.width).toBe('25%');
    expect((bar.root.querySelector('.clip-note') as HTMLElement).hidden).toBe(true);
    expect(bar.root.querySelector('.clip-live')?.getAttribute('href')).toBe('http://host/');
    expect(injected(bar.root)).toBe(0);
    bar.update({ ...model, progress: 7, note: 'The end of this clip is still to come.' });
    expect((bar.root.querySelector('.clip-fill') as HTMLElement).style.width).toBe('100%');
    expect((bar.root.querySelector('.clip-note') as HTMLElement).hidden).toBe(false);
  });

  it('calls its handlers, and shows the state of the pin', () => {
    const calls: string[] = [];
    const bar = new ClipBar(
      { toggle: () => calls.push('toggle'), restart: () => calls.push('restart'), pin: () => calls.push('pin') },
      'http://host/?clip=5-0',
      'http://host/',
    );
    bar.update(model);
    const buttons = [...bar.root.querySelectorAll('button')];
    const byText = (text: string) => buttons.find((node) => node.textContent === text);
    byText('Pause')?.click();
    byText('Restart')?.click();
    byText('Keep this clip')?.click();
    expect(calls).toEqual(['toggle', 'restart', 'pin']);
    bar.update({ ...model, playing: false, pin: 'pinned' });
    expect(byText('Play')).toBeDefined();
    const kept = buttons.find((node) => node.textContent === 'Kept');
    expect(kept?.disabled).toBe(true);
    // A server that pins for nobody: no button.
    bar.update({ ...model, pin: 'off' });
    expect(kept?.hidden).toBe(true);
  });
});
