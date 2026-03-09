# Create Wayland::Scanner imported target for cross-compilation
# This uses the HOST wayland-scanner binary (not the cross-compiled one)
if(NOT TARGET Wayland::Scanner)
    add_executable(Wayland::Scanner IMPORTED GLOBAL)
    set_target_properties(Wayland::Scanner PROPERTIES
        IMPORTED_LOCATION "/usr/bin/wayland-scanner"
    )
    set(WaylandScanner_FOUND TRUE)
endif()
