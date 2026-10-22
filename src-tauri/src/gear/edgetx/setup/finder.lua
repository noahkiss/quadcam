-- QuadCam telemetry screen: finder 1
-- Written by QuadCam for a 128x64 EdgeTX radio. QuadCam replaces this file when its setup
-- changes; edit the setup in QuadCam, not this file.
-- Shows the receiver's signal (1RSS, dBm) in large digits and the link quality (RQly).
-- While the finder control (@SWITCH@) is on and this screen shows, it beeps: slowly and low
-- at -100 dBm, fast and high at -40 dBm. No telemetry, no beeps.

local SWITCH = "@switch@"
local ON_SIGN = @sign@             -- +1: on reads +1024; -1: on reads -1024
local WEAK, STRONG = -100, -40     -- dBm
local SLOW, FAST = 150, 10         -- 10 ms ticks between beeps
local LOW_HZ, HIGH_HZ = 800, 2000
local BEEP_MS = 40

local switchId
local nextBeep = 0
local lastRead = 0
local signal, quality, linked, on = 0, 0, false, false
local period, tone = SLOW, LOW_HZ

local function init()
  local info = getFieldInfo(SWITCH)
  switchId = info and info.id or nil
end

local function read(now)
  lastRead = now
  signal = getValue("1RSS") or 0
  quality = getValue("RQly") or 0
  on = switchId ~= nil and ((getValue(switchId) or 0) * ON_SIGN) > 512
  linked = signal ~= 0 and getRSSI() ~= 0
  if not linked then return end
  local f = (signal - WEAK) / (STRONG - WEAK)
  if f < 0 then f = 0 elseif f > 1 then f = 1 end
  period = math.floor(SLOW - f * (SLOW - FAST) + 0.5)
  tone = math.floor(LOW_HZ + f * (HIGH_HZ - LOW_HZ) + 0.5)
end

local function run(event)
  local now = getTime()
  if lastRead == 0 or now - lastRead >= 10 then read(now) end
  if linked and on then
    if now >= nextBeep then
      playTone(tone, BEEP_MS, 0)
      nextBeep = now + period
    end
  else
    nextBeep = 0
  end
  lcd.clear()
  lcd.drawText(0, 0, "FINDER", SMLSIZE + INVERS)
  lcd.drawText(104, 0, "1RSS", SMLSIZE)
  if linked then
    if signal < 0 then lcd.drawText(0, 18, "-", DBLSIZE) end
    lcd.drawNumber(14, 8, math.abs(signal), XXLSIZE)
    lcd.drawText(104, 26, "dBm", SMLSIZE)
    lcd.drawText(0, 56, string.format("LQ %d%%", quality), SMLSIZE)
    if on then
      lcd.drawText(70, 56, string.format("beep %d.%ds", math.floor(period / 100), math.floor((period % 100) / 10)), SMLSIZE)
    else
      lcd.drawText(70, 56, "@SWITCH@ on=beep", SMLSIZE)
    end
  else
    lcd.drawText(10, 20, "NO TLM", DBLSIZE)
    lcd.drawText(0, 56, "no signal, no beeps", SMLSIZE)
  end
  return 0
end

return { init = init, run = run }
