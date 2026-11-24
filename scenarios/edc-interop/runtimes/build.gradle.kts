// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

// Shared settings of the four EDC runtimes. Each is one runnable jar
// ("shadow" jar), started with EDC's BaseRuntime.

plugins {
    java
    // builds the one-file runnable jar, as the EDC launchers do
    id("com.gradleup.shadow") version "9.6.1" apply false
}

subprojects {
    apply(plugin = "java")
    apply(plugin = "application")
    apply(plugin = "com.gradleup.shadow")

    java {
        toolchain {
            languageVersion.set(JavaLanguageVersion.of(21))
        }
    }

    // Jackson 2 at a version without the known advisories (osv.dev, 2026-11-21):
    // EDC RC8 and its Swagger modules ask for 2.19.2 and 2.22.1, and the newest
    // version asked for wins, so this only raises the version, never lowers it.
    dependencies {
        "implementation"(platform("com.fasterxml.jackson:jackson-bom:2.22.3"))
    }

    extensions.configure<JavaApplication> {
        mainClass.set("org.eclipse.edc.boot.system.runtime.BaseRuntime")
    }

    tasks.named<com.github.jengelman.gradle.plugins.shadow.tasks.ShadowJar>("shadowJar") {
        // EDC finds its extensions through META-INF/services files
        mergeServiceFiles()
        duplicatesStrategy = DuplicatesStrategy.INCLUDE
        archiveFileName.set("${project.name}.jar")
    }
}
