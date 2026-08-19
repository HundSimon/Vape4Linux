cmake_minimum_required(VERSION 3.21)
project(Vape421Native VERSION 1.0 LANGUAGES C)

if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
    message(FATAL_ERROR "Only 64-bit Minecraft/JVM targets are supported")
endif()

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

set(VAPE421_JAVA_HOME "$ENV{JAVA_HOME}" CACHE PATH
        "JDK containing JNI/JVMTI headers")
if(NOT VAPE421_JAVA_HOME)
    if(WIN32)
        set(_vape421_java_candidates
                "C:/Program Files/Java/jdk1.8.0_301"
                "C:/Program Files/Java/jdk-17")
    else()
        set(_vape421_java_candidates
                "/usr/lib/jvm/default"
                "/usr/lib/jvm/default-runtime"
                "/usr/lib/jvm/java-21-openjdk")
    endif()
    foreach(candidate IN LISTS _vape421_java_candidates)
        if(EXISTS "${candidate}/include/jni.h")
            set(VAPE421_JAVA_HOME "${candidate}")
            break()
        endif()
    endforeach()
endif()

find_path(VAPE421_JNI_INCLUDE jni.h
        HINTS "${VAPE421_JAVA_HOME}/include" REQUIRED)
if(WIN32)
    set(_vape421_jni_platform win32)
else()
    set(_vape421_jni_platform linux)
endif()
find_path(VAPE421_JNI_MD_INCLUDE jni_md.h
        HINTS "${VAPE421_JAVA_HOME}/include/${_vape421_jni_platform}" REQUIRED)
find_path(VAPE421_JVMTI_INCLUDE jvmti.h
        HINTS "${VAPE421_JAVA_HOME}/include" REQUIRED)

set(VAPE421_PRODUCT_JAR "" CACHE FILEPATH
        "Recovered product JAR loaded by the native agent")
if(NOT VAPE421_PRODUCT_JAR OR NOT EXISTS "${VAPE421_PRODUCT_JAR}")
    message(FATAL_ERROR
            "VAPE421_PRODUCT_JAR must name an existing recovered product JAR")
endif()
get_filename_component(VAPE421_PRODUCT_JAR
        "${VAPE421_PRODUCT_JAR}" ABSOLUTE)

if(WIN32)
    if(NOT MSVC)
        message(FATAL_ERROR "The Windows bridge requires the MSVC x64 toolchain")
    endif()
    enable_language(RC)
    file(TO_CMAKE_PATH "${VAPE421_PRODUCT_JAR}" VAPE421_PRODUCT_JAR_RC_PATH)
    configure_file(payload.rc.in generated/payload.rc @ONLY)
    set_property(SOURCE "${CMAKE_CURRENT_BINARY_DIR}/generated/payload.rc"
            APPEND PROPERTY OBJECT_DEPENDS "${VAPE421_PRODUCT_JAR}")
    set(_vape421_platform_sources
            loader_bootstrap.c
            "${CMAKE_CURRENT_BINARY_DIR}/generated/payload.rc")
else()
    set(_vape421_platform_sources loader_bootstrap_posix.c)
endif()

add_library(Vape421Native SHARED
        dllmain.c
        native_bridge.c
        ${_vape421_platform_sources})
target_include_directories(Vape421Native PRIVATE
        "${CMAKE_CURRENT_SOURCE_DIR}"
        "${VAPE421_JNI_INCLUDE}"
        "${VAPE421_JNI_MD_INCLUDE}"
        "${VAPE421_JVMTI_INCLUDE}")

if(WIN32)
    target_compile_definitions(Vape421Native PRIVATE
            WIN32_LEAN_AND_MEAN _CRT_SECURE_NO_WARNINGS)
    target_compile_options(Vape421Native PRIVATE /W4)
    target_link_libraries(Vape421Native PRIVATE user32 ws2_32)

    add_executable(Vape421Injector injector.c)
    target_compile_definitions(Vape421Injector PRIVATE
            WIN32_LEAN_AND_MEAN _CRT_SECURE_NO_WARNINGS)
    target_compile_options(Vape421Injector PRIVATE /W4)
    set_property(TARGET Vape421Native Vape421Injector PROPERTY
            MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL")
else()
    find_package(Threads REQUIRED)
    target_compile_options(Vape421Native PRIVATE
            -Wall -Wextra -Werror=implicit-function-declaration)
    target_link_libraries(Vape421Native PRIVATE Threads::Threads ${CMAKE_DL_LIBS})
    set_target_properties(Vape421Native PROPERTIES
            BUILD_RPATH "" INSTALL_RPATH "")
endif()

set_target_properties(Vape421Native PROPERTIES
        RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/dist"
        LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/dist")
if(TARGET Vape421Injector)
    set_target_properties(Vape421Injector PROPERTIES
            RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/dist")
endif()

foreach(configuration Debug Release RelWithDebInfo MinSizeRel)
    string(TOUPPER "${configuration}" configuration_upper)
    set_target_properties(Vape421Native PROPERTIES
            RUNTIME_OUTPUT_DIRECTORY_${configuration_upper}
            "${CMAKE_BINARY_DIR}/dist"
            LIBRARY_OUTPUT_DIRECTORY_${configuration_upper}
            "${CMAKE_BINARY_DIR}/dist")
    if(TARGET Vape421Injector)
        set_target_properties(Vape421Injector PROPERTIES
                RUNTIME_OUTPUT_DIRECTORY_${configuration_upper}
                "${CMAKE_BINARY_DIR}/dist")
    endif()
endforeach()

include(CTest)
if(BUILD_TESTING AND WIN32)
    add_executable(Vape421BootstrapTest
            tests/loader_bootstrap_test.c loader_bootstrap.c)
    target_include_directories(Vape421BootstrapTest PRIVATE
            "${CMAKE_CURRENT_SOURCE_DIR}")
    target_compile_definitions(Vape421BootstrapTest PRIVATE
            WIN32_LEAN_AND_MEAN _CRT_SECURE_NO_WARNINGS)
    target_compile_options(Vape421BootstrapTest PRIVATE /W4)
    target_link_libraries(Vape421BootstrapTest PRIVATE ws2_32)
    add_test(NAME Vape421BootstrapStandalone
            COMMAND Vape421BootstrapTest standalone)
    add_test(NAME Vape421BootstrapHandoff COMMAND Vape421BootstrapTest)
    add_test(NAME Vape421BootstrapRejectsInvalid
            COMMAND Vape421BootstrapTest invalid)
endif()
