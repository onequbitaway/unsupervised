// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { panelFacts } from '../state/profile';
import { addItems, advance, emptyTicker } from '../state/ticker';
import type { Profile } from '../state/types';
import { Controls } from './controls';
import { button, el, setShown, setText } from './dom';
import { Hud } from './hud';
import { keyAction, wheelFactor } from './input';
import { cssColor, Legend } from './legend';
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
