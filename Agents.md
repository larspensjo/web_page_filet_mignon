# Instructions to consider

## Bugs
* Is there a lessons learned here? A design issue? Lack of robustness? That is, I want to find similar problems and prevent future problems of the same type.
* When fixing bugs, make sure to add a unit test that locks functionality in and prevents it from happening again.

## Structs
* Prefer private members to enforce encapsulation.
*   Avoid simple getters or setters, if possible. Instead, strive to actually do the work inside the struct.
* Pure data containers can still be public.

## Testing
* Consider using dependency injection and mock objects to enhance unit testing

## General Rust design
* mod.rs and lib.rs should be thin wrappers.
