# Application description

## Typical workflow
A typical workflow would to look like this:
* I am using a news aggregating site in a regular web browser.
* Every time I find an interesting news entry, I will copy the URL, one at a time, to web_page_filet_mignon. Sometimes, more than one.
* After every added URL, I want web_page_filet_mignon to immediately update the jobs.
* If web_page_filet_mignon was in a finished state when I add more URLs, I want it to go back to active state again.
* When done adding URLs, I press the Archive button that exports everything.
* The user will sometimes shut down the application, restart it, and continue where they left off.

## Display

The application shall consist of the following main components, from top to bottom:
* A rate limiting progress bar.
* A URL drop box left of a window with a treeview of URLs being downloaded or completed. The drop box shall have a fixed width, but the treeview should expand horisontally with the main window.
* A Stop / Finish button
* An Archive button. The button shall be disabled if no web sites are downloaded. It shall be disabled after the archive was generated, but enabled as soon as another web site has been downloaded.
* The button shall be placed horisontally after each other. They shall have fixed width.

### Progress bar
I want a progress bar at the top of the window.

It should show the number of tokens total, as related to a max limit. The max limit shall be 200000.

That way, while you continue to add more URLs, the progress bar increase.

## Artifacts
### Archive
The archive is a single text file with all downloaded markdown files. Every file shall be prefixed by a header that makes it easy to detect.

### Persistence
* If the application is shut down and restarted, it shall recover the list of web sites that were completely downloaded.
* Web sites that were partially downloaded will restart from scratch, so we don't need to save partially downloaded web sites (just the URL).
