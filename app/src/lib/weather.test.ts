import { describe, expect, it } from "vitest";
import { compass, imperial, SUN_LABEL, weatherLine } from "./weather";

describe("weather", () => {
  const w = { hour: "2026-09-28T18:00Z", temperature_c: 18.4, wind_kmh: 20, gusts_kmh: 32, wind_from_deg: 315, source: "archive" };

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
});
