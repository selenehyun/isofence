// Should detect: global state mutations
globalThis.myApp = { version: "1.0" };
process.env.NODE_ENV = "test";
window.DEBUG = true;

export const name = "side-effects";
