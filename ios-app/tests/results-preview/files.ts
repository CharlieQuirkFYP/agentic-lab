export const Paths = { cache: 'illustrative-preview://cache' };
export class File {
  uri: string;
  constructor(...parts: string[]) { this.uri = parts.join('/'); }
  write(content: string) { (globalThis as any).__resultsPreviewExport = { uri: this.uri, content }; }
}
