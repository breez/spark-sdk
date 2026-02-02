plugins {
    `java-gradle-plugin`
    kotlin("jvm")
    `maven-publish`
}

gradlePlugin {
    plugins {
        create("breezSdkSpark") {
            id = "technology.breez.spark.kmp"
            implementationClass = "technology.breez.spark.BreezSdkSparkPlugin"
        }
    }
}

dependencies {
    compileOnly("org.jetbrains.kotlin:kotlin-gradle-plugin:1.9.21")
    compileOnly("org.jetbrains.kotlin:kotlin-gradle-plugin-api:1.9.21")
}

val libraryVersion: String by project

group = "technology.breez.spark"
version = libraryVersion

publishing {
    repositories {
        maven {
            name = "breezReposilite"
            url = uri("https://mvn.breez.technology/releases")
            credentials(PasswordCredentials::class)
            authentication {
                create<BasicAuthentication>("basic")
            }
        }
    }

    publications {
        this.forEach {
            (it as MavenPublication).apply {
                pom {
                    name.set("breez-sdk-spark-kmp-plugin")
                    description.set("Gradle plugin that auto-configures iOS framework search paths for the Breez Spark SDK KMP bindings.")
                    url.set("https://breez.technology")
                    licenses {
                        license {
                            name.set("MIT")
                            url.set("https://github.com/breez/spark-sdk/blob/main/LICENSE")
                        }
                    }
                    scm {
                        connection.set("scm:git:github.com/breez/spark-sdk.git")
                        developerConnection.set("scm:git:ssh://github.com/breez/spark-sdk.git")
                        url.set("https://github.com/breez/spark-sdk")
                    }
                }
            }
        }
    }
}
