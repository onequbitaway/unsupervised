import { describe, expect, it } from 'vitest';
import { kindLabel, siteLabel } from './sites';

describe('siteLabel', () => {
  it('is the name of the site', () => {
    expect(siteLabel({ name: 'Okafor Bakery', kind: 'shop' })).toBe('Okafor Bakery');
  });

  it('is the kind in words when the site has no name', () => {
    expect(siteLabel({ name: '', kind: 'police_station' })).toBe('police station');
    expect(siteLabel({ name: ' \n ', kind: 'city_hall' })).toBe('city hall');
    expect(siteLabel({ name: '', kind: '' })).toBe('site');
    expect(kindLabel('police_station')).toBe('police station');
  });

  it('cleans and cuts a name that an owner chose', () => {
    const label = siteLabel({ name: 'A\u202e<b>shop</b>\nwith a very long name that goes on', kind: 'shop' });
    expect(label).toBe('A<b>shop</b> with a v…');
    expect(label).toHaveLength(22);
  });
});
