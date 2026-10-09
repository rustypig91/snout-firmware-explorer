cmake_minimum_required(VERSION 3.29)
project(sensor_monitor_fixture LANGUAGES C)

if(NOT CMAKE_C_COMPILER_ID MATCHES "^(GNU|Clang|TI|TIClang)$")
    message(FATAL_ERROR "Supported fixture compilers: GNU Arm GCC, LLVM Clang, TI armcl and TI tiarmclang")
endif()

option(FIXTURE_CUSTOM_SECTIONS "Exercise long custom output-section names" OFF)

# CMake's TIClang rule adds --rom_model and startup/runtime support. These
# freestanding analysis images have their own entry and intentionally no CRT.
if(CMAKE_C_COMPILER_ID STREQUAL "TIClang")
    set(CMAKE_C_LINK_EXECUTABLE
        "<CMAKE_C_COMPILER> <FLAGS> -o <TARGET> <LINK_FLAGS> <OBJECTS> <LINK_LIBRARIES>")
endif()

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

if(FIXTURE_CUSTOM_SECTIONS)
    list(APPEND FIXTURE_SOURCES src/custom_sections.c)
endif()

function(add_fixture_objects target extra_slots)
    add_library(${target} OBJECT ${FIXTURE_SOURCES})
    target_compile_definitions(${target} PRIVATE EXTRA=${extra_slots})
    if(CMAKE_C_COMPILER_ID STREQUAL "TI")
        target_compile_options(${target} PRIVATE
            --gcc --opt_level=off --symdebug:dwarf --symdebug:dwarf_version=4
            --emit_warnings_as_errors --display_error_number
        )
    else()
        target_compile_options(${target} PRIVATE
            -mcpu=cortex-m3 -mthumb -ffreestanding -fno-builtin -fno-common
            -fno-unwind-tables -fno-asynchronous-unwind-tables -O0 -g
            -Wall -Wextra -Werror
        )
        if(CMAKE_C_COMPILER_ID STREQUAL "TIClang")
            target_compile_options(${target} PRIVATE -gdwarf-3)
        else()
            target_compile_options(${target} PRIVATE -gdwarf-4 -fstack-usage)
        endif()
    endif()
endfunction()

function(add_fixture_image target objects)
    add_executable(${target} $<TARGET_OBJECTS:${objects}>)
    set_target_properties(${target} PROPERTIES SUFFIX ".elf")
    if(CMAKE_C_COMPILER_ID MATCHES "^(TI|TIClang)$")
        set(linker_script "${CMAKE_CURRENT_SOURCE_DIR}/src/cortex-m-ti.cmd")
        if(CMAKE_C_COMPILER_ID STREQUAL "TI")
            target_link_options(${target} PRIVATE
                "${linker_script}" --entry_point=Reset_Handler --unused_section_elimination=off
                "--map_file=${CMAKE_CURRENT_BINARY_DIR}/${target}.map"
            )
        else()
            target_link_options(${target} PRIVATE
                -nostdlib "-Wl,${linker_script}" -Wl,--entry_point=Reset_Handler
                -Wl,--unused_section_elimination=off
                "-Wl,--map_file=${CMAKE_CURRENT_BINARY_DIR}/${target}.map"
            )
        endif()
    else()
        set(linker_script "${CMAKE_CURRENT_SOURCE_DIR}/src/cortex-m.ld")
        target_link_options(${target} PRIVATE
            -mcpu=cortex-m3 -mthumb -nostdlib -Wl,--build-id=none
            -Wl,--cref "-Wl,-T,${linker_script}"
            "-Wl,-Map,${CMAKE_CURRENT_BINARY_DIR}/${target}.map"
        )
        if(CMAKE_C_COMPILER_ID STREQUAL "Clang")
            # Clang consumes --no-demangle; the single-dash spelling reaches lld.
            target_link_options(${target} PRIVATE -fuse-ld=lld -Wl,-no-demangle)
        else()
            target_link_options(${target} PRIVATE -Wl,--no-demangle)
        endif()
    endif()
    set_target_properties(${target} PROPERTIES LINK_DEPENDS "${linker_script}")
endfunction()

# Baseline and stripped images share exactly the same compiled objects.
add_fixture_objects(cortex-m-objects 0)
add_fixture_image(cortex-m cortex-m-objects)
add_fixture_image(cortex-m-stripped cortex-m-objects)
if(CMAKE_C_COMPILER_ID MATCHES "^(TI|TIClang)$")
    # Strip the finished ELF, including debug sections, rather than using TI's
    # deprecated --no_symtable (which leaves DWARF behind).
    find_program(FIXTURE_STRIP arm-none-eabi-strip REQUIRED)
    add_custom_command(TARGET cortex-m-stripped POST_BUILD
        COMMAND "${FIXTURE_STRIP}" --strip-all "$<TARGET_FILE:cortex-m-stripped>"
        VERBATIM
    )
else()
    target_link_options(cortex-m-stripped PRIVATE -Wl,--strip-all)
endif()

# The comparison target reserves eight additional sample slots.
add_fixture_objects(cortex-m-grown-objects 8)
add_fixture_image(cortex-m-grown cortex-m-grown-objects)
