cmake_minimum_required(VERSION 3.20)
project(sensor_monitor_fixture LANGUAGES C)

set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

set(FIXTURE_SOURCES
    src/main.c
    src/config.c
    src/sensor.c
    src/diag.c
    src/telemetry.c
    src/transport.c
)

function(add_fixture_objects target extra_slots)
    add_library(${target} OBJECT ${FIXTURE_SOURCES})
    target_compile_definitions(${target} PRIVATE EXTRA=${extra_slots})
    target_compile_options(${target} PRIVATE
        -mcpu=cortex-m3
        -mthumb
        -ffreestanding
        -fno-builtin
        -fno-common
        -fno-unwind-tables
        -fno-asynchronous-unwind-tables
        -O0
        -g
        -gdwarf-4
        -fstack-usage
        -Wall
        -Wextra
        -Werror
    )
endfunction()

function(add_fixture_image target objects)
    add_executable(${target} $<TARGET_OBJECTS:${objects}>)
    set_target_properties(${target} PROPERTIES
        SUFFIX ".elf"
        LINK_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/src/cortex-m.ld"
    )
    target_link_options(${target} PRIVATE
        -mcpu=cortex-m3
        -mthumb
        -nostdlib
        -Wl,--build-id=none
        "-Wl,-T,${CMAKE_CURRENT_SOURCE_DIR}/src/cortex-m.ld"
        "-Wl,-Map,${CMAKE_CURRENT_BINARY_DIR}/${target}.map"
    )
    if(CMAKE_C_COMPILER_ID STREQUAL "GNU")
        target_link_options(${target} PRIVATE -Wl,--cref,--no-demangle)
    else()
        target_link_options(${target} PRIVATE -fuse-ld=lld)
    endif()
endfunction()

# Baseline and stripped images share exactly the same compiled objects.
add_fixture_objects(cortex-m-objects 0)
add_fixture_image(cortex-m cortex-m-objects)
add_fixture_image(cortex-m-stripped cortex-m-objects)
target_link_options(cortex-m-stripped PRIVATE -Wl,--strip-all)

# The comparison target reserves eight additional sample slots.
add_fixture_objects(cortex-m-grown-objects 8)
add_fixture_image(cortex-m-grown cortex-m-grown-objects)
