// Lets TypeScript accept the stylesheet import in main.tsx, and the font files the fonts module
// imports by URL (Vite gives the URL of the file it copies into the build).
declare module '*.css';

declare module '*.woff2?url' {
  const url: string;
  export default url;
}
