// Shared constants for the whole site. Every tunable number appears here,
// with its reasoning, exactly once, so a module never hard-codes a limit
// another module also depends on.

export const SITE_ORIGIN = "https://omnibin.dev";
export const REPO = "https://github.com/fzakaria/omnibin";

// The two sibling sites. Every store path here exists in both, so a version
// row links to the package in the index it came from and to a map of the
// closure it pulls.
export const MULTIVERSE_URL = "https://nixmultiverse.com/";
export const SEENIX_URL = "https://seenix.dev/";
export const TRYNIX_URL = "https://trynix.dev/";

export const STORE_DIR = "/nix/store";

// Systems with a published database. The first is what a page shows before
// anybody picks, because it is the one with the widest coverage.
export const SYSTEMS = ["x86_64-linux", "aarch64-linux"];

export const VIEWS = ["commands", "stats"];

// How many search hits to render. The index is 51,000 names and a query of
// "e" matches most of them; past a couple of hundred rows nobody is reading,
// they are just laying out.
export const MAX_RESULTS = 200;

// Below this many characters a search matches too much to be worth running.
export const MIN_QUERY = 2;

export const COPY_FLASH_MS = 1200;

export const HTTP_NOT_FOUND = 404;

// What a data fetch resolves to when it fails. A sentinel rather than null,
// because "still loading" and "will never load" render differently and both
// have to be distinguishable from data.
export const SHARD_ERROR = "error";
