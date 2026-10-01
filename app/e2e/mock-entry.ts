// Bundled by e2e/global-setup.ts and injected before any page script.
import { installMock } from "../src/ipc/mock/install";

installMock(window.__QC_MOCK__);
