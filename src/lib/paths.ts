export function joinNativePath(parent: string, child: string): string {
  return `${parent.replace(/[\\/]+$/, "")}/${child}`;
}

export function fileNameFromPath(path: string | null): string | null {
  if (!path) {
    return null;
  }
  return path.split(/[\\/]/).pop() ?? path;
}

export function deriveDefaultInputDir(defaultOutputDir: string): string {
  if (!defaultOutputDir) {
    return "Desktop/@StorageSlim/input";
  }
  if (/[\\/]output$/i.test(defaultOutputDir)) {
    return defaultOutputDir.replace(/[\\/]output$/i, (match) => match.replace(/output/i, "input"));
  }
  return `${defaultOutputDir.replace(/[\\/]?$/, "")}/input`;
}
