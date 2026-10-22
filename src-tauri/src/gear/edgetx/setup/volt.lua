-- QuadCam telemetry screen: volt 1
-- Written by QuadCam for a 128x64 EdgeTX radio. QuadCam replaces this file when its setup
-- changes; edit the setup in QuadCam, not this file.
-- Shows the quad's battery (RxBt) in large digits, the TX power (TPWR), the link quality
-- (RQly) and Timer @timer_number@.

local TIMER = @timer@

local function clock(s)
  local sign = ""
  if s < 0 then sign, s = "-", -s end
  return string.format("%s%02d:%02d", sign, math.floor(s / 60), s % 60)
end

local function run(event)
  lcd.clear()
  local volts = getValue("RxBt") or 0
  lcd.drawText(0, 0, "QUAD", SMLSIZE)
  if volts > 0 then
    lcd.drawNumber(4, 8, math.floor(volts * 100 + 0.5), XXLSIZE + PREC2)
    lcd.drawText(98, 26, "V", DBLSIZE)
  else
    lcd.drawText(12, 14, "--.--", DBLSIZE)
    lcd.drawText(12, 32, "no RxBt", SMLSIZE)
  end
  lcd.drawText(90, 0, string.format("%dmW", getValue("TPWR") or 0), SMLSIZE)
  lcd.drawText(0, 50, string.format("LQ%d%%", getValue("RQly") or 0), MIDSIZE)
  local t = model.getTimer(TIMER)
  lcd.drawText(74, 50, clock((t and t.value) or 0), MIDSIZE)
  return 0
end

return { run = run }
