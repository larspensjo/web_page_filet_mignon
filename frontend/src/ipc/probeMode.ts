export function isProbePage(search = window.location.search): boolean {
	return new URLSearchParams(search).has("probe");
}
