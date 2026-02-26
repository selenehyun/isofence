// Should detect: static class field with mutable init
export class Registry {
  static instances = new Map<string, Registry>();
  static config = { debug: false };

  // Safe — primitive
  static VERSION = "1.0.0";
  static MAX = 100;
}

// Safe class — no mutable static fields
export class Utils {
  static readonly NAME = "utils";

  static format(value: string): string {
    return value.trim();
  }
}
