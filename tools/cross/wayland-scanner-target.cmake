# Create Wayland::Scanner imported target for cross-compilation
# This uses the HOST wayland-scanner binary (not the cross-compiled one)
if(NOT TARGET Wayland::Scanner)
    add_executable(Wayland::Scanner IMPORTED GLOBAL)
    set_target_properties(Wayland::Scanner PROPERTIES
        IMPORTED_LOCATION "/usr/bin/wayland-scanner"
    )
    set(WaylandScanner_FOUND TRUE)
endif()

# Load KF6/Plasma stub targets for cross-compilation
set(_veridian_stubs_file "${CMAKE_CURRENT_LIST_DIR}/../../../target/veridian-sysroot/usr/lib/cmake/veridian-kf6-stubs.cmake")
if(NOT EXISTS "${_veridian_stubs_file}")
    # Try relative to sysroot from CMAKE_PREFIX_PATH
    foreach(_prefix IN LISTS CMAKE_PREFIX_PATH)
        if(EXISTS "${_prefix}/lib/cmake/veridian-kf6-stubs.cmake")
            set(_veridian_stubs_file "${_prefix}/lib/cmake/veridian-kf6-stubs.cmake")
            break()
        endif()
    endforeach()
endif()
if(EXISTS "${_veridian_stubs_file}")
    include("${_veridian_stubs_file}")
endif()
unset(_veridian_stubs_file)
