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

## Logging
* Use the `engine_logging` crate for all logging. Import macros: `use engine_logging::{engine_info, engine_warn, engine_error};`
* Available macros: `engine_trace!`, `engine_debug!`, `engine_info!`, `engine_warn!`, `engine_error!`
* Default log level is INFO (debug messages are filtered out)
* Logs are written to both terminal and `./engine.log` in the current working directory
* Log errors with context: include the URL, job_id, or other identifying information
* In unit tests, call `engine_logging::initialize_for_tests();` to enable logging output
