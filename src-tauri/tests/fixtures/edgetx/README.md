# EdgeTX card fixtures

| Folder | Holds |
|---|---|
| `gx12-factory/` | A RadioMaster GX12's factory card files (EdgeTX 2.11.0): `RADIO/radio.yml` and `MODELS/model00.yml`, CRLF as the radio wrote them. Registration IDs are blanked and `globalTimer` is 0. The model holds the 2.11 custom-switch keys (`functionSwitchConfig` and the LED colour blocks) that EdgeTX 2.12 still reads |

Every other EdgeTX card in the tests is synthetic (`gear::edgetx::synth`).
