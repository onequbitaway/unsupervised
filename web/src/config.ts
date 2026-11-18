// Every number that a designer could want to tune in the viewer.

export const CONFIG = {
  feed: {
    /** The version of the feed protocol that this viewer speaks. */
    protocol: 1,
    /** Shortest wait before the feed connects again, in milliseconds. */
    reconnectMinMs: 500,
    /** Longest wait before the feed connects again. */
    reconnectMaxMs: 15_000,
    /** Time between two pings. */
    pingMs: 10_000,
    /** A feed without any message for this long is closed and opened again. */
    staleMs: 30_000,
    /** Zones that the viewer subscribes to at most. The server allows 32. */
    maxZones: 16,
  },
  motion: {
    /** Tick length that the viewer assumes before it has measured one. */
    defaultTickMs: 1_000,
    /** Shortest and longest tick length that a measurement can give. */
    minTickMs: 40,
    maxTickMs: 5_000,
    /** Weight of a new measurement of the tick length, 0 to 1. */
    tickSmoothing: 0.25,
    /** A move lasts this many ticks, so that the next move comes before it ends. */
    glideTicks: 1.1,
    /** A move of more tiles than this is a jump: the resident is there at once. */
    snapTiles: 40,
  },
  text: {
    /** Longest name of a resident, a site or a city on the screen. */
    nameMax: 22,
    /** Longest speech in a bubble. */
    speechMax: 84,
    /** Longest line in the news ticker. */
    headlineMax: 120,
    /** Longest name of a model. */
    modelMax: 24,
  },
  speech: {
    /** Time that a speech bubble stays. */
    holdMs: 6_000,
    /** Speech bubbles on the screen at most. */
    maxBubbles: 28,
  },
  ticker: {
    /** Events that the ticker remembers. */
    maxItems: 160,
    /** Time that one event stays in the main line. */
    showMs: 5_000,
    /** Time that a headline stays in the main line. */
    headlineMs: 9_000,
    /** Lowest drama score of a headline. */
    headlineDrama: 71,
    /** Lowest drama score of a notable event. */
    notableDrama: 11,
    /** Drama points that an event loses for each minute of age. */
    agePenaltyPerMinute: 12,
    /** An event older than this is never shown. */
    maxAgeMs: 10 * 60_000,
    /** Time between two requests for notable events from the whole world. */
    pollMs: 8_000,
    /** Lines in the list of recent events. */
    recentLines: 3,
  },
  view: {
    /** Part of the screen that the whole country fills. */
    countryFill: 0.72,
    /** Part of the screen that a city fills. */
    cityFill: 0.92,
    /** Part of the smaller side of the screen that one district fills. */
    streetFill: 1.05,
    /** How much closer than the street view the camera is when it follows a resident. */
    followZoom: 1.5,
    /** Greatest zoom, in pixels for one tile. */
    maxScale: 56,
    /** Zones whose map the viewer draws at most. */
    maxMapZones: 25,
    /** Time in which the camera covers half of the way to its target. */
    easeHalfLifeMs: 140,
    /** Zoom factor of one step of the mouse wheel or of a button. */
    zoomStep: 1.25,
    /** Tiles that one press of an arrow key moves, as a part of the screen. */
    panStep: 0.18,
    /** A zone with this many residents has the hottest colour, also in a small world. */
    heatFloor: 20,
    /** Part of a zone that stays empty at its border on the heat map. */
    heatGap: 0.03,
    /** A pointer that moves less than this many pixels between down and up is a click. */
    clickPx: 6,
  },
  render: {
    /** Pixels for one tile in the texture of a district. */
    tilePx: 12,
    /** Height of a resident figure in tiles. */
    figureTiles: 1.5,
    /** Smallest height of a resident on the screen, in pixels. */
    minFigurePx: 5,
    /** Below this many pixels for one tile a resident is a dot. */
    dotBelowPx: 6,
    /** Names of residents on the screen at most. */
    nameBudget: 150,
    /** Time between two choices of the residents that get a name. */
    nameEveryMs: 250,
    /** Below this many pixels for one tile no names and no site labels. */
    labelsBelowPx: 7,
    /** A click this near to a resident selects it, in pixels. */
    pickPx: 22,
    /** Smallest and largest size of a dot for a far resident, in pixels. */
    dotMinPx: 6,
    dotMaxPx: 9,
    /** Smallest and largest size of an activity marker, in pixels. */
    markerMinPx: 11,
    markerMaxPx: 26,
    /** Time that one step of a walking figure lasts. */
    stepMs: 170,
    /** District maps that the viewer paints in one frame at most. */
    paintsPerFrame: 1,
    /** Height of the screen for which the label sizes below are meant. */
    labelBaseHeight: 900,
    /** Smallest and largest factor on the label sizes for other screens. */
    labelScaleMin: 0.85,
    labelScaleMax: 1.7,
    /** Sizes of the labels on a screen of the base height, in pixels. */
    nameFontPx: 14,
    siteFontPx: 15,
    districtFontPx: 17,
    cityFontPx: 27,
    bubbleFontPx: 15,
    /** Greatest width of a speech bubble, in pixels at the base height. */
    bubbleWidthPx: 230,
    /** Time in which a speech bubble fades away. */
    bubbleFadeMs: 400,
    /** Site labels on the screen at most. */
    siteLabelBudget: 90,
    /** A site label is shown when it is not wider than its site times this. */
    siteLabelOverflow: 1.7,
    /** How opaque the roof of a site is at the city level and at the street level. */
    roofCity: 0.92,
    roofStreet: 0.16,
  },
  ui: {
    /**
     * Time between two updates of the slow parts: the head and the panel of
     * the page, and the choice of the zones that the screen needs.
     */
    everyMs: 200,
  },
  panel: {
    /** Time between two reads of the profile of the selected resident. */
    refreshMs: 4_000,
    /** Time between two reads while the viewer follows a resident that the feed does not show. */
    searchMs: 1_000,
  },
  maps: {
    /** Maps that the viewer reads at the same time at most. */
    parallel: 4,
    /** Time after which the viewer reads the map of a district on the screen again. */
    refreshMs: 60_000,
    /** Time before the viewer asks again for a map that the server refused. */
    retryMs: 3_000,
  },
  start: {
    /** Shortest and longest wait before the viewer asks the server again for the world. */
    retryMinMs: 500,
    retryMaxMs: 8_000,
  },
  stats: {
    /** Frames that the speed numbers cover. */
    samples: 600,
  },
} as const;
