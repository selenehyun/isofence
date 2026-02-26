// Should detect: prototype mutations
Array.prototype.customMethod = function() { return this; };
String.prototype.toTitle = function() { return this; };

export const marker = true;
