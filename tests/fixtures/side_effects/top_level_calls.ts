// Should detect: top-level function calls
initializeApp();
console.log("module loaded");
setupMiddleware();

export function handler() {
  return "ok";
}
