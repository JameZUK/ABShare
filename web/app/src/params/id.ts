// Book and group IDs (26 characters of base32): route parameters must match exactly, so a
// crafted link (`#/book/..%2F..%2Fv1%2Fadmin`) can't steer API calls elsewhere (review W1).
import type { ParamMatcher } from '@sveltejs/kit';

export const match: ParamMatcher = (param) => /^[a-z2-7]{26}$/.test(param);
