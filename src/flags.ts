// src/flags.ts
// Maps a server's remark/host to a country flag emoji. On Apple platforms
// (our targets) these render as real flag glyphs. Matching is keyword-based
// because subscription remarks are free-form ("Amsterdam", "NL-1", "🇳🇱 AMS").
// Falls back to a globe when nothing matches — never guesses wrong.

const RULES: [RegExp, string][] = [
  [/(netherland|amsterdam|\bnl\b|\bams\b)/i, "🇳🇱"],
  [/(german|frankfurt|berlin|munich|\bde\b|\bfra\b)/i, "🇩🇪"],
  [/(finland|helsinki|\bfi\b|\bhel\b)/i, "🇫🇮"],
  [/(united states|america|\busa?\b|new york|\bnyc?\b|los angeles|\blax\b|miami|seattle|dallas|ashburn)/i, "🇺🇸"],
  [/(united kingdom|england|britain|london|\buk\b|\blon\b)/i, "🇬🇧"],
  [/(france|paris|\bfr\b|\bpar\b)/i, "🇫🇷"],
  [/(canada|toronto|montreal|vancouver|\bca\b|\byyz\b)/i, "🇨🇦"],
  [/(sweden|stockholm|\bse\b|\bsto\b)/i, "🇸🇪"],
  [/(japan|tokyo|osaka|\bjp\b|\bnrt\b|\bhnd\b)/i, "🇯🇵"],
  [/(singapore|\bsg\b|\bsin\b)/i, "🇸🇬"],
  [/(poland|warsaw|\bpl\b|\bwaw\b)/i, "🇵🇱"],
  [/(turkey|türkiye|istanbul|\btr\b|\bist\b)/i, "🇹🇷"],
  [/(russia|moscow|\bspb\b|\bru\b|\bmsk\b)/i, "🇷🇺"],
  [/(switzerland|zurich|geneva|\bch\b|\bzrh\b)/i, "🇨🇭"],
  [/(spain|madrid|barcelona|\bes\b|\bmad\b)/i, "🇪🇸"],
  [/(italy|milan|rome|\bit\b|\bmxp\b)/i, "🇮🇹"],
  [/(hong ?kong|\bhk\b|\bhkg\b)/i, "🇭🇰"],
  [/(korea|seoul|\bkr\b|\bicn\b)/i, "🇰🇷"],
  [/(austria|vienna|\bat\b|\bvie\b)/i, "🇦🇹"],
  [/(norway|oslo|\bno\b|\bosl\b)/i, "🇳🇴"],
  [/(denmark|copenhagen|\bdk\b|\bcph\b)/i, "🇩🇰"],
  [/(latvia|riga|\blv\b|\brix\b)/i, "🇱🇻"],
  [/(lithuania|vilnius|\blt\b)/i, "🇱🇹"],
  [/(estonia|tallinn|\bee\b)/i, "🇪🇪"],
  [/(ireland|dublin|\bie\b|\bdub\b)/i, "🇮🇪"],
  [/(belgium|brussels|\bbe\b|\bbru\b)/i, "🇧🇪"],
  [/(czech|prague|\bcz\b|\bprg\b)/i, "🇨🇿"],
  [/(romania|bucharest|\bro\b)/i, "🇷🇴"],
  [/(india|mumbai|delhi|\bin\b|\bbom\b)/i, "🇮🇳"],
  [/(australia|sydney|melbourne|\bau\b|\bsyd\b)/i, "🇦🇺"],
  [/(brazil|são paulo|sao paulo|\bbr\b|\bgru\b)/i, "🇧🇷"],
  [/(kazakhstan|almaty|astana|\bkz\b)/i, "🇰🇿"],
  [/(ukraine|kyiv|kiev|\bua\b)/i, "🇺🇦"],
  [/(luxembourg|\blu\b)/i, "🇱🇺"],
  [/(hungary|budapest|\bhu\b)/i, "🇭🇺"],
  [/(portugal|lisbon|\bpt\b)/i, "🇵🇹"],
  [/(united arab|dubai|\buae\b|\bae\b)/i, "🇦🇪"],
  [/(israel|tel aviv|\bil\b)/i, "🇮🇱"],
];

/**
 * Flag emoji for a server. Checks the remark first (usually a city/country),
 * then the hostname, then gives up with a globe.
 */
export function flagFor(remark: string, host = ""): string {
  // If the remark already contains a flag emoji, keep it.
  const existing = remark.match(/\p{Regional_Indicator}\p{Regional_Indicator}/u);
  if (existing) return existing[0];

  const haystack = `${remark} ${host}`;
  for (const [re, flag] of RULES) {
    if (re.test(haystack)) return flag;
  }
  return "🌐";
}

/**
 * Whether this system draws regional-indicator pairs as flags at all.
 *
 * Windows does not: Segoe UI Emoji has no flag glyphs, so WebView2 draws the
 * two letters ("DE", "GB") boxed and squeezed into the 28 px flag column. The
 * Countries list leaves the column out there; the name says the country.
 */
export function rendersFlagEmoji(userAgent: string): boolean {
  return !/\bWindows\b/.test(userAgent);
}
