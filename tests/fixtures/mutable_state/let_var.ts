// Should detect: let/var at module scope
let counter = 0;
var globalFlag = true;

export function getCount() {
  return counter++;
}

export function isEnabled() {
  return globalFlag;
}
