// Account IDs (positive integers): route parameters must match exactly, as for book IDs, so a
// crafted link can't steer API calls elsewhere (review W1).
import type { ParamMatcher } from '@sveltejs/kit';

export const match: ParamMatcher = (param) => /^[1-9][0-9]{0,9}$/.test(param);
