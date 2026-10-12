// Sun tags and weather as the library shows them.
import type { IconName } from "../components/Icon";
import type { Sky, SunTag, Weather } from "../ipc/types";

export const SUN_LABEL: Record<SunTag, string> = {
  dawn: "Dawn",
  sunrise: "Sunrise",
  golden_hour: "Golden hour",
  sunset: "Sunset",
  dusk: "Dusk",
};

const POINTS = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

/** A wind direction in degrees as one of eight compass points. */
export function compass(deg: number): string {
  return POINTS[Math.round((((deg % 360) + 360) % 360) / 45) % 8];
}

/** Fahrenheit and mph where the locale's region uses them. */
export function imperial(locale = navigator.language): boolean {
  const region = (() => {
    try {
      return new Intl.Locale(locale).maximize().region;
    } catch {
      return undefined;
    }
  })();
  return region === "US" || region === "LR" || region === "MM";
}

/** `18 °C · wind 12 km/h from NW, gusts 20`. */
export function weatherLine(w: Weather, us = imperial()): string {
  const t = us ? `${Math.round((w.temperature_c * 9) / 5 + 32)} °F` : `${Math.round(w.temperature_c)} °C`;
  const speed = (kmh: number) => Math.round(us ? kmh / 1.609344 : kmh);
  const unit = us ? "mph" : "km/h";
  let wind = `wind ${speed(w.wind_kmh)} ${unit}`;
  if (w.wind_from_deg != null) wind += ` from ${compass(w.wind_from_deg)}`;
  if (w.gusts_kmh != null) wind += `, gusts ${speed(w.gusts_kmh)}`;
  return `${t} · ${wind}`;
}

export const SKY_LABEL: Record<Sky, string> = {
  clear: "Clear",
  partly_cloudy: "Partly cloudy",
  cloudy: "Cloudy",
  fog: "Fog",
  drizzle: "Drizzle",
  rain: "Rain",
  heavy_rain: "Heavy rain",
  sleet: "Sleet",
  snow: "Snow",
  thunderstorm: "Thunderstorm",
  hail: "Hail",
};

/** The icon for a sky; a clear or partly cloudy night shows the moon. */
export function skyIcon(sky: Sky, isDay: boolean | null): IconName {
  const night = isDay === false;
  switch (sky) {
    case "clear":
      return night ? "moon" : "sun";
    case "partly_cloudy":
      return night ? "cloud-moon" : "cloud-sun";
    case "cloudy":
      return "cloud";
    case "fog":
      return "fog";
    case "drizzle":
      return "cloud-drizzle";
    case "rain":
      return "cloud-rain";
    case "heavy_rain":
      return "cloud-heavy-rain";
    case "sleet":
      return "cloud-sleet";
    case "snow":
      return "cloud-snow";
    case "thunderstorm":
      return "cloud-bolt";
    case "hail":
      return "cloud-hail";
  }
}

/** The sky values a record has, as label and value rows. Precipitation only above 0;
 * visibility in km, or mi where the region uses it. */
export function skyFacts(w: Weather, us = imperial()): [string, string][] {
  const rows: [string, string][] = [];
  if (w.cloud_cover_pct != null) rows.push(["Cloud cover", `${Math.round(w.cloud_cover_pct)}%`]);
  if (w.precipitation_mm != null && w.precipitation_mm > 0) rows.push(["Precipitation", `${w.precipitation_mm.toFixed(1)} mm`]);
  if (w.humidity_pct != null) rows.push(["Humidity", `${Math.round(w.humidity_pct)}%`]);
  if (w.visibility_m != null) {
    const v = us ? w.visibility_m / 1609.344 : w.visibility_m / 1000;
    rows.push(["Visibility", `${v >= 10 ? Math.round(v) : v.toFixed(1)} ${us ? "mi" : "km"}`]);
  }
  return rows;
}

/** A record saved before QuadCam asked for the sky; the core looks it up once more. */
export function lacksSky(w: Weather): boolean {
  return w.cloud_cover_pct == null && w.weather_code == null;
}
