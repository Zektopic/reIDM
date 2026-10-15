# dmgbuild settings for reIDM's disk image: a window with reIDM.app on the left, the
# Applications folder on the right, and a background with an arrow between them. make-dmg.sh
# passes the paths with -D. dmgbuild writes the window's layout (.DS_Store) itself, without
# scripting Finder, so this works on a build machine with no one logged in.
#
# The icon positions must match APP and APPS in tools/make_dmg_background.py, which draws the
# background around them.
import os.path

application = defines["app"]  # noqa: F821 — dmgbuild provides `defines`
appname = os.path.basename(application)

format = "UDZO"
filesystem = "HFS+"
size = None  # dmgbuild sizes the image to fit

files = [application]
symlinks = {"Applications": "/Applications"}
icon = defines["volume_icon"]  # noqa: F821
hide_extensions = [appname]

background = defines["background"]  # noqa: F821
window_rect = ((200, 140), (660, 400))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False
include_icon_view_settings = True

arrange_by = None
grid_offset = (0, 0)
grid_spacing = 100
label_pos = "bottom"
text_size = 13
icon_size = 112
icon_locations = {
    appname: (180, 196),
    "Applications": (480, 196),
}
