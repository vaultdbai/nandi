/** `https://github.com/...`: an address the GitHub sign-in works with */
export function isGithubUrl(url: string): boolean {
  return /^https:\/\/(www\.)?github\.com\//i.test(url.trim())
}
