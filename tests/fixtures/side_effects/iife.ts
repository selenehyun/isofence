// Should detect: IIFEs at module scope
(function() {
  console.log("IIFE executed");
})();

(() => {
  console.log("Arrow IIFE executed");
})();

export const value = 42;
