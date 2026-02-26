// This module should be completely safe — no hazards
export type User = {
  id: string;
  name: string;
};

export interface Config {
  apiUrl: string;
  timeout: number;
}

export const MAX_RETRIES = 3;
export const API_URL = "https://api.example.com";

export function add(a: number, b: number): number {
  return a + b;
}

export function formatName(first: string, last: string): string {
  return `${first} ${last}`;
}

export class Formatter {
  format(value: string): string {
    return value.trim();
  }
}

export enum Status {
  Active = "active",
  Inactive = "inactive",
}
