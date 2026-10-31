export function findTexts(): string[];
export function review(dict: Record<string, string>, texts: string[]): { missing: string[]; stale: string[]; broken: string[] };
