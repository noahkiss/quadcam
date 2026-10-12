// Sun tags and weather as the library shows them.
import type { SunTag, Weather } from "../ipc/types";

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
