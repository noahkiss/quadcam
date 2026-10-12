import { describe, expect, it } from "vitest";
import type { Sky } from "../ipc/types";
import { compass, imperial, lacksSky, SKY_LABEL, skyFacts, skyIcon, SUN_LABEL, weatherLine } from "./weather";

describe("weather", () => {
  const old = { hour: "2026-09-28T18:00Z", temperature_c: 18.4, wind_kmh: 20, gusts_kmh: 32, wind_from_deg: 315, source: "archive" };
  const none = { cloud_cover_pct: null, precipitation_mm: null, rain_mm: null, snowfall_cm: null, weather_code: null, sky: null, humidity_pct: null, is_day: null, visibility_m: null };
  const w = { ...old, ...none };
  const sky = { ...w, cloud_cover_pct: 40, precipitation_mm: 0.4, weather_code: 2, sky: "partly_cloudy" as Sky, humidity_pct: 71, is_day: true, visibility_m: 24000 };

  it("names the wind's compass point", () => {
    expect(compass(0)).toBe("N");
    expect(compass(359)).toBe("N");
    expect(compass(200)).toBe("S");
    expect(compass(-45)).toBe("NW");
  });

  it("shows metric, or imperial where the region uses it", () => {
    expect(weatherLine(w, false)).toBe("18 °C · wind 20 km/h from NW, gusts 32");
    expect(weatherLine(w, true)).toBe("65 °F · wind 12 mph from NW, gusts 20");
    expect(weatherLine({ ...w, gusts_kmh: null, wind_from_deg: null }, false)).toBe("18 °C · wind 20 km/h");
    expect(imperial("en-US")).toBe(true);
    expect(imperial("en-GB")).toBe(false);
    expect(imperial("de")).toBe(false);
  });

  it("labels every sun tag", () => {
    expect(SUN_LABEL.golden_hour).toBe("Golden hour");
    expect(Object.keys(SUN_LABEL)).toHaveLength(5);
  });

  it("labels every sky and gives each an icon, the moon on a clear night", () => {
    expect(Object.keys(SKY_LABEL)).toHaveLength(11);
    expect(SKY_LABEL.heavy_rain).toBe("Heavy rain");
    expect(skyIcon("clear", true)).toBe("sun");
    expect(skyIcon("clear", false)).toBe("moon");
    expect(skyIcon("clear", null)).toBe("sun");
    expect(skyIcon("partly_cloudy", false)).toBe("cloud-moon");
    expect(skyIcon("rain", false)).toBe("cloud-rain");
    expect(new Set((Object.keys(SKY_LABEL) as Sky[]).map((k) => skyIcon(k, true))).size).toBe(11);
  });

  it("lists the sky values a record has", () => {
    expect(skyFacts(sky, false)).toEqual([
      ["Cloud cover", "40%"],
      ["Precipitation", "0.4 mm"],
      ["Humidity", "71%"],
      ["Visibility", "24 km"],
    ]);
    expect(skyFacts({ ...sky, visibility_m: 8000 }, true).at(-1)).toEqual(["Visibility", "5.0 mi"]);
    expect(skyFacts({ ...sky, precipitation_mm: 0, visibility_m: null }, false).map(([k]) => k)).toEqual(["Cloud cover", "Humidity"]);
    expect(skyFacts(w, false)).toEqual([]);
  });

  it("knows a record saved before the sky values", () => {
    expect(lacksSky(w)).toBe(true);
    expect(lacksSky(sky)).toBe(false);
    expect(lacksSky({ ...w, weather_code: 0 })).toBe(false);
  });
});
