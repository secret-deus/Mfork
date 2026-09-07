// Coarse geolocation for the overview globe: proxy nodes are placed from
// region hints in their names (country codes, Chinese names, flag emoji),
// the local machine from its IANA time zone. [lon, lat].

export type LonLat = [number, number];

const CITY: Record<string, LonLat> = {
  hk: [114.17, 22.3],
  tw: [121.56, 25.03],
  jp: [139.69, 35.69],
  kr: [126.98, 37.57],
  sg: [103.82, 1.35],
  my: [101.69, 3.14],
  th: [100.5, 13.75],
  vn: [106.7, 10.78],
  ph: [120.98, 14.6],
  id: [106.85, -6.21],
  in: [72.88, 19.08],
  ae: [55.27, 25.2],
  tr: [28.98, 41.01],
  ru: [37.62, 55.76],
  de: [8.68, 50.11],
  fr: [2.35, 48.86],
  gb: [-0.13, 51.51],
  nl: [4.9, 52.37],
  se: [18.07, 59.33],
  fi: [24.94, 60.17],
  ch: [8.54, 47.38],
  it: [12.5, 41.9],
  es: [-3.7, 40.42],
  pl: [21.01, 52.23],
  ua: [30.52, 50.45],
  us_la: [-118.24, 34.05],
  us_sj: [-121.89, 37.34],
  us_sea: [-122.33, 47.61],
  us_ny: [-74.01, 40.71],
  us_dal: [-96.8, 32.78],
  us_chi: [-87.63, 41.88],
  ca: [-79.38, 43.65],
  mx: [-99.13, 19.43],
  br: [-46.63, -23.55],
  ar: [-58.38, -34.6],
  cl: [-70.67, -33.45],
  au: [151.21, -33.87],
  nz: [174.76, -36.85],
  za: [28.05, -26.2],
  eg: [31.24, 30.04],
  il: [34.78, 32.08],
  cn_sh: [121.47, 31.23],
  cn_bj: [116.4, 39.9],
  cn_gz: [113.26, 23.13],
  cn_sz: [114.06, 22.54],
  mo: [113.54, 22.2],
};

// order matters: more specific hints first
const HINTS: [RegExp, keyof typeof CITY][] = [
  [/香港|hong\s*kong|🇭🇰|\bhkg?\b/i, "hk"],
  [/台湾|台灣|taiwan|台北|🇹🇼|\btwn?\b/i, "tw"],
  [/日本|japan|东京|東京|大阪|🇯🇵|\bjpn?\b|tokyo|osaka/i, "jp"],
  [/韩国|韓國|korea|首尔|首爾|🇰🇷|\bkor?\b|seoul/i, "kr"],
  [/新加坡|singapore|狮城|獅城|🇸🇬|\bsgp?\b/i, "sg"],
  [/马来|馬來|malaysia|🇲🇾|\bmys?\b/i, "my"],
  [/泰国|泰國|thailand|曼谷|🇹🇭|\btha?\b/i, "th"],
  [/越南|vietnam|🇻🇳|\bvnm?\b/i, "vn"],
  [/菲律宾|菲律賓|philippines|马尼拉|🇵🇭|\bphl?\b/i, "ph"],
  [/印尼|印度尼西亚|indonesia|雅加达|🇮🇩|\bidn?\b/i, "id"],
  [/印度|india|孟买|🇮🇳|\bind?\b/i, "in"],
  [/阿联酋|迪拜|dubai|🇦🇪|\bare\b|\buae\b/i, "ae"],
  [/土耳其|turkey|伊斯坦布尔|🇹🇷|\btur?\b/i, "tr"],
  [/俄罗斯|俄羅斯|russia|莫斯科|🇷🇺|\brus?\b/i, "ru"],
  [/德国|德國|germany|法兰克福|法蘭克福|🇩🇪|\bdeu?\b|frankfurt/i, "de"],
  [/法国|法國|france|巴黎|🇫🇷|\bfra?\b|paris/i, "fr"],
  [/英国|英國|britain|united\s*kingdom|伦敦|倫敦|🇬🇧|\bgbr?\b|\buk\b|london/i, "gb"],
  [/荷兰|荷蘭|netherlands|阿姆斯特丹|🇳🇱|\bnld?\b/i, "nl"],
  [/瑞典|sweden|🇸🇪|\bswe?\b/i, "se"],
  [/芬兰|芬蘭|finland|🇫🇮|\bfin?\b/i, "fi"],
  [/瑞士|switzerland|苏黎世|🇨🇭|\bche?\b/i, "ch"],
  [/意大利|italy|🇮🇹|\bita?\b/i, "it"],
  [/西班牙|spain|🇪🇸|\besp?\b/i, "es"],
  [/波兰|波蘭|poland|🇵🇱|\bpol?\b/i, "pl"],
  [/乌克兰|烏克蘭|ukraine|🇺🇦|\bukr?\b/i, "ua"],
  [/洛杉矶|洛杉磯|los\s*angeles|\bla\b|\blax\b/i, "us_la"],
  [/圣何塞|聖何塞|san\s*jose|硅谷|矽谷|\bsjc\b/i, "us_sj"],
  [/西雅图|西雅圖|seattle|\bsea\b/i, "us_sea"],
  [/纽约|紐約|new\s*york|\bnyc?\b/i, "us_ny"],
  [/达拉斯|達拉斯|dallas|\bdfw\b/i, "us_dal"],
  [/芝加哥|chicago|\bord\b/i, "us_chi"],
  [/美国|美國|america|united\s*states|🇺🇸|\busa?\b/i, "us_la"],
  [/加拿大|canada|多伦多|🇨🇦|\bcan?\b/i, "ca"],
  [/墨西哥|mexico|🇲🇽|\bmex?\b/i, "mx"],
  [/巴西|brazil|圣保罗|🇧🇷|\bbra?\b/i, "br"],
  [/阿根廷|argentina|🇦🇷|\barg?\b/i, "ar"],
  [/智利|chile|🇨🇱|\bchl\b/i, "cl"],
  [/澳大利亚|澳洲|australia|悉尼|🇦🇺|\baus?\b|sydney/i, "au"],
  [/新西兰|紐西蘭|new\s*zealand|🇳🇿|\bnzl?\b/i, "nz"],
  [/南非|south\s*africa|🇿🇦|\bzaf?\b/i, "za"],
  [/埃及|egypt|🇪🇬|\begy?\b/i, "eg"],
  [/以色列|israel|🇮🇱|\bisr?\b/i, "il"],
  [/澳门|澳門|macau|macao|🇲🇴|\bmo\b/i, "mo"],
  [/上海|shanghai/i, "cn_sh"],
  [/北京|beijing/i, "cn_bj"],
  [/广州|廣州|guangzhou/i, "cn_gz"],
  [/深圳|shenzhen/i, "cn_sz"],
  [/中国|中國|china|🇨🇳|\bchn\b|\bcn\b/i, "cn_sh"],
];

const NAME: Record<keyof typeof CITY, string> = {
  hk: "香港", tw: "台北", jp: "东京", kr: "首尔", sg: "新加坡", my: "吉隆坡", th: "曼谷", vn: "胡志明市",
  ph: "马尼拉", id: "雅加达", in: "孟买", ae: "迪拜", tr: "伊斯坦布尔", ru: "莫斯科", de: "法兰克福",
  fr: "巴黎", gb: "伦敦", nl: "阿姆斯特丹", se: "斯德哥尔摩", fi: "赫尔辛基", ch: "苏黎世", it: "罗马",
  es: "马德里", pl: "华沙", ua: "基辅", us_la: "洛杉矶", us_sj: "圣何塞", us_sea: "西雅图", us_ny: "纽约",
  us_dal: "达拉斯", us_chi: "芝加哥", ca: "多伦多", mx: "墨西哥城", br: "圣保罗", ar: "布宜诺斯艾利斯",
  cl: "圣地亚哥", au: "悉尼", nz: "奥克兰", za: "约翰内斯堡", eg: "开罗", il: "特拉维夫", cn_sh: "上海",
  cn_bj: "北京", cn_gz: "广州", cn_sz: "深圳", mo: "澳门",
};

export interface Place {
  pos: LonLat;
  region: string;
}

export function locateNodeDetail(name: string): Place | null {
  for (const [re, key] of HINTS) {
    if (re.test(name)) return { pos: CITY[key], region: NAME[key] };
  }
  return null;
}

export function locateSelfDetail(): Place {
  let tz = "";
  try {
    tz = Intl.DateTimeFormat().resolvedOptions().timeZone ?? "";
  } catch {
    /* ignore */
  }
  for (const [re, key] of TZ) {
    if (re.test(tz)) return { pos: CITY[key], region: NAME[key] };
  }
  return { pos: CITY.cn_sh, region: NAME.cn_sh };
}

export function locateNode(name: string): LonLat | null {
  for (const [re, key] of HINTS) {
    if (re.test(name)) return CITY[key];
  }
  return null;
}

const TZ: [RegExp, keyof typeof CITY][] = [
  [/Asia\/Shanghai|Asia\/Chongqing|Asia\/Urumqi|Asia\/Harbin|PRC/, "cn_sh"],
  [/Asia\/Hong_Kong/, "hk"],
  [/Asia\/Macau/, "mo"],
  [/Asia\/Taipei/, "tw"],
  [/Asia\/Tokyo/, "jp"],
  [/Asia\/Seoul/, "kr"],
  [/Asia\/Singapore/, "sg"],
  [/Asia\/Kuala_Lumpur/, "my"],
  [/Asia\/Bangkok/, "th"],
  [/Asia\/(Ho_Chi_Minh|Saigon)/, "vn"],
  [/Asia\/Manila/, "ph"],
  [/Asia\/Jakarta/, "id"],
  [/Asia\/(Kolkata|Calcutta)/, "in"],
  [/Asia\/Dubai/, "ae"],
  [/Europe\/(Istanbul)|Asia\/Istanbul/, "tr"],
  [/Europe\/Moscow/, "ru"],
  [/Europe\/Berlin/, "de"],
  [/Europe\/Paris/, "fr"],
  [/Europe\/London/, "gb"],
  [/Europe\/Amsterdam/, "nl"],
  [/Europe\/Stockholm/, "se"],
  [/Europe\/Helsinki/, "fi"],
  [/Europe\/Zurich/, "ch"],
  [/Europe\/Rome/, "it"],
  [/Europe\/Madrid/, "es"],
  [/Europe\/Warsaw/, "pl"],
  [/Europe\/Kyiv|Europe\/Kiev/, "ua"],
  [/America\/Los_Angeles|America\/Vancouver/, "us_la"],
  [/America\/New_York|America\/Detroit/, "us_ny"],
  [/America\/Chicago/, "us_chi"],
  [/America\/Toronto/, "ca"],
  [/America\/Mexico_City/, "mx"],
  [/America\/Sao_Paulo/, "br"],
  [/America\/(Argentina|Buenos_Aires)/, "ar"],
  [/America\/Santiago/, "cl"],
  [/Australia\//, "au"],
  [/Pacific\/Auckland/, "nz"],
  [/Africa\/Johannesburg/, "za"],
  [/Africa\/Cairo/, "eg"],
  [/Asia\/(Jerusalem|Tel_Aviv)/, "il"],
];

export function locateSelf(): LonLat {
  let tz = "";
  try {
    tz = Intl.DateTimeFormat().resolvedOptions().timeZone ?? "";
  } catch {
    /* ignore */
  }
  for (const [re, key] of TZ) {
    if (re.test(tz)) return CITY[key];
  }
  return CITY.cn_sh;
}
