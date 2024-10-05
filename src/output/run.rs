// (c) Meta Platforms, Inc. and affiliates.
//
// Use of this source code is governed by an MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT.

use std::env;
use std::sync::atomic;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::Map;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::output as tv;
use crate::spec;
use tv::step::TestStep;
use tv::{config, dut, emitter, error, log, run, state};

/// The outcome of a TestRun.
/// It's returned when the scope method of the [`TestRun`] object is used.
pub struct TestRunOutcome {
    /// Reports the execution status of the test
    pub status: spec::TestStatus,
    /// Reports the result of the test
    pub result: spec::TestResult,
}

/// The main diag test run.
///
/// This object describes a single run instance of the diag, and therefore drives the test session.
pub struct TestRun {
    name: String,
    version: String,
    parameters: Map<String, Value>,
    dut: dut::DutInfo,
    command_line: String,
    metadata: Option<Map<String, Value>>,
    state: Arc<Mutex<state::TestState>>,
}

impl TestRun {
    /// Creates a new [`TestRunBuilder`] object.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ocptv::output::*;
    ///
    /// let dut = DutInfo::builder("my_dut").build();
    /// let builder = TestRun::builder("run_name", &dut, "1.0");
    /// ```
    pub fn builder(name: &str, dut: &dut::DutInfo, version: &str) -> TestRunBuilder {
        TestRunBuilder::new(name, dut, version)
    }

    /// Creates a new [`TestRun`] object.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ocptv::output::*;
    ///
    /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0");
    /// ```
    pub fn new(name: &str, dut_id: &str, version: &str) -> TestRun {
        let dut = dut::DutInfo::new(dut_id);
        TestRunBuilder::new(name, &dut, version).build()
    }

    /// Starts the test run.
    ///
    /// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#schemaversion
    /// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#testrunstart
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// # use ocptv::output::*;
    ///
    /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0");
    /// run.start().await?;
    ///
    /// # Ok::<(), WriterError>(())
    /// # });
    /// ```
    pub async fn start(self) -> Result<StartedTestRun, emitter::WriterError> {
        let version = SchemaVersion::new();
        self.state
            .lock()
            .await
            .emitter
            .emit(&version.to_artifact())
            .await?;

        let mut builder = run::TestRunStart::builder(
            &self.name,
            &self.version,
            &self.command_line,
            &self.parameters,
            &self.dut,
        );

        if let Some(m) = &self.metadata {
            for m in m {
                builder = builder.add_metadata(m.0, m.1.clone())
            }
        }

        let start = builder.build();
        self.state
            .lock()
            .await
            .emitter
            .emit(&start.to_artifact())
            .await?;

        Ok(StartedTestRun::new(self))
    }

    // disabling this for the moment so we don't publish api that's unusable.
    // see: https://github.com/rust-lang/rust/issues/70263
    //
    // /// Builds a scope in the [`TestRun`] object, taking care of starting and
    // /// ending it. View [`TestRun::start`] and [`TestRun::end`] methods.
    // /// After the scope is constructed, additional objects may be added to it.
    // /// This is the preferred usage for the [`TestRun`], since it guarantees
    // /// all the messages are emitted between the start and end messages, the order
    // /// is respected and no messages is lost.
    // ///
    // /// # Examples
    // ///
    // /// ```rust
    // /// # tokio_test::block_on(async {
    // /// # use ocptv::output::*;
    // ///
    // /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0");
    // /// run.scope(|r| async {
    // ///     r.log(LogSeverity::Info, "First message").await?;
    // ///     Ok(TestRunOutcome {
    // ///         status: TestStatus::Complete,
    // ///         result: TestResult::Pass,
    // ///     })
    // /// }).await?;
    // ///
    // /// # Ok::<(), WriterError>(())
    // /// # });
    // /// ```
    // pub async fn scope<F, R>(self, func: F) -> Result<(), emitters::WriterError>
    // where
    //     R: Future<Output = Result<TestRunOutcome, emitters::WriterError>>,
    //     for<'a> F: Fut2<'a, R>,
    // {
    //     let run = self.start().await?;
    //     let outcome = func(&run).await?;
    //     run.end(outcome.status, outcome.result).await?;

    //     Ok(())
    // }
}

/// Builder for the [`TestRun`] object.
pub struct TestRunBuilder {
    name: String,
    dut: dut::DutInfo,
    version: String,
    parameters: Map<String, Value>,
    command_line: String,
    metadata: Option<Map<String, Value>>,
    config: Option<config::Config>,
}

impl TestRunBuilder {
    pub fn new(name: &str, dut: &dut::DutInfo, version: &str) -> Self {
        Self {
            name: name.to_string(),
            dut: dut.clone(),
            version: version.to_string(),
            parameters: Map::new(),
            command_line: env::args().collect::<Vec<_>>()[1..].join(" "),
            metadata: None,
            config: None,
        }
    }

    /// Adds a user defined parameter to the future [`TestRun`] object.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ocptv::output::*;
    ///
    /// let dut = DutInfo::builder("dut_id").build();
    /// let run = TestRunBuilder::new("run_name", &dut, "1.0")
    ///     .add_parameter("param1", "value1".into())
    ///     .build();
    /// ```
    pub fn add_parameter(mut self, key: &str, value: Value) -> TestRunBuilder {
        self.parameters.insert(key.to_string(), value.clone());
        self
    }

    /// Adds the command line used to run the test session  to the future
    /// [`TestRun`] object.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ocptv::output::*;
    ///
    /// let dut = DutInfo::builder("dut_id").build();
    /// let run = TestRunBuilder::new("run_name", &dut, "1.0")
    ///     .command_line("my_diag --arg value")
    ///     .build();
    /// ```
    pub fn command_line(mut self, cmd: &str) -> TestRunBuilder {
        self.command_line = cmd.to_string();
        self
    }

    /// Adds the configuration for the test session to the future [`TestRun`] object
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ocptv::output::{Config, TestRunBuilder, DutInfo};
    ///
    /// let dut = DutInfo::builder("dut_id").build();
    /// let run = TestRunBuilder::new("run_name", &dut, "1.0")
    ///     .config(Config::builder().build())
    ///     .build();
    /// ```
    pub fn config(mut self, value: config::Config) -> TestRunBuilder {
        self.config = Some(value);
        self
    }

    /// Adds user defined metadata to the future [`TestRun`] object
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ocptv::output::*;
    ///
    /// let dut = DutInfo::builder("dut_id").build();
    /// let run = TestRunBuilder::new("run_name", &dut, "1.0")
    ///     .add_metadata("meta1", "value1".into())
    ///     .build();
    /// ```
    pub fn add_metadata(mut self, key: &str, value: Value) -> TestRunBuilder {
        self.metadata = match self.metadata {
            Some(mut metadata) => {
                metadata.insert(key.to_string(), value.clone());
                Some(metadata)
            }
            None => {
                let mut metadata = Map::new();
                metadata.insert(key.to_string(), value.clone());
                Some(metadata)
            }
        };
        self
    }

    pub fn build(self) -> TestRun {
        let config = self.config.unwrap_or(config::Config::builder().build());
        let emitter = emitter::JsonEmitter::new(config.timezone, config.writer);
        let state = state::TestState::new(emitter);
        TestRun {
            name: self.name,
            dut: self.dut,
            version: self.version,
            parameters: self.parameters,
            command_line: self.command_line,
            metadata: self.metadata,
            state: Arc::new(Mutex::new(state)),
        }
    }
}

/// A test run that was started.
///
/// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#testrunstart
pub struct StartedTestRun {
    run: TestRun,

    step_seqno: atomic::AtomicU64,
}

impl StartedTestRun {
    fn new(run: TestRun) -> StartedTestRun {
        StartedTestRun {
            run,
            step_seqno: atomic::AtomicU64::new(0),
        }
    }
    /// Ends the test run.
    ///
    /// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#testrunend
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// # use ocptv::output::*;
    ///
    /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0").start().await?;
    /// run.end(TestStatus::Complete, TestResult::Pass).await?;
    ///
    /// # Ok::<(), WriterError>(())
    /// # });
    /// ```
    pub async fn end(
        &self,
        status: spec::TestStatus,
        result: spec::TestResult,
    ) -> Result<(), emitter::WriterError> {
        let end = run::TestRunEnd::builder()
            .status(status)
            .result(result)
            .build();

        let emitter = &self.run.state.lock().await.emitter;

        emitter.emit(&end.to_artifact()).await?;
        Ok(())
    }

    /// Emits a Log message.
    /// This method accepts a [`models::LogSeverity`] to define the severity
    /// and a [`std::string::String`] for the message.
    ///
    /// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#log
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// # use ocptv::output::*;
    ///
    /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0").start().await?;
    /// run.log(
    ///     LogSeverity::Info,
    ///     "This is a log message with INFO severity",
    /// ).await?;
    /// run.end(TestStatus::Complete, TestResult::Pass).await?;
    ///
    /// # Ok::<(), WriterError>(())
    /// # });
    /// ```
    pub async fn log(
        &self,
        severity: spec::LogSeverity,
        msg: &str,
    ) -> Result<(), emitter::WriterError> {
        let log = log::Log::builder(msg).severity(severity).build();

        let emitter = &self.run.state.lock().await.emitter;

        let artifact = spec::TestRunArtifact {
            artifact: spec::TestRunArtifactDescendant::Log(log.to_artifact()),
        };
        emitter
            .emit(&spec::RootArtifact::TestRunArtifact(artifact))
            .await?;

        Ok(())
    }

    /// Emits a Log message.
    /// This method accepts a [`objects::Log`] object.
    ///
    /// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#log
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// # use ocptv::output::*;
    ///
    /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0").start().await?;
    /// run.log_with_details(
    ///     &Log::builder("This is a log message with INFO severity")
    ///         .severity(LogSeverity::Info)
    ///         .source("file", 1)
    ///         .build(),
    /// ).await?;
    /// run.end(TestStatus::Complete, TestResult::Pass).await?;
    ///
    /// # Ok::<(), WriterError>(())
    /// # });
    /// ```
    pub async fn log_with_details(&self, log: &log::Log) -> Result<(), emitter::WriterError> {
        let emitter = &self.run.state.lock().await.emitter;

        let artifact = spec::TestRunArtifact {
            artifact: spec::TestRunArtifactDescendant::Log(log.to_artifact()),
        };
        emitter
            .emit(&spec::RootArtifact::TestRunArtifact(artifact))
            .await?;

        Ok(())
    }

    /// Emits a Error message.
    /// This method accepts a [`std::string::String`] to define the symptom.
    ///
    /// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#error
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// # use ocptv::output::*;
    ///
    /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0").start().await?;
    /// run.error("symptom").await?;
    /// run.end(TestStatus::Complete, TestResult::Pass).await?;
    ///
    /// # Ok::<(), WriterError>(())
    /// # });
    /// ```
    pub async fn error(&self, symptom: &str) -> Result<(), emitter::WriterError> {
        let error = error::Error::builder(symptom).build();
        let emitter = &self.run.state.lock().await.emitter;

        let artifact = spec::TestRunArtifact {
            artifact: spec::TestRunArtifactDescendant::Error(error.to_artifact()),
        };
        emitter
            .emit(&spec::RootArtifact::TestRunArtifact(artifact))
            .await?;

        Ok(())
    }

    /// Emits a Error message.
    /// This method accepts a [`std::string::String`] to define the symptom and
    /// another [`std::string::String`] as error message.
    ///
    /// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#error
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// # use ocptv::output::*;
    ///
    /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0").start().await?;
    /// run.error_with_msg("symptom", "error messasge").await?;
    /// run.end(TestStatus::Complete, TestResult::Pass).await?;
    ///
    /// # Ok::<(), WriterError>(())
    /// # });
    /// ```
    pub async fn error_with_msg(
        &self,
        symptom: &str,
        msg: &str,
    ) -> Result<(), emitter::WriterError> {
        let error = error::Error::builder(symptom).message(msg).build();
        let emitter = &self.run.state.lock().await.emitter;

        let artifact = spec::TestRunArtifact {
            artifact: spec::TestRunArtifactDescendant::Error(error.to_artifact()),
        };
        emitter
            .emit(&spec::RootArtifact::TestRunArtifact(artifact))
            .await?;

        Ok(())
    }

    /// Emits a Error message.
    /// This method acceps a [`objects::Error`] object.
    ///
    /// ref: https://github.com/opencomputeproject/ocp-diag-core/tree/main/json_spec#error
    ///
    /// # Examples
    ///
    /// ```rust
    /// # tokio_test::block_on(async {
    /// # use ocptv::output::*;
    ///
    /// let run = TestRun::new("diagnostic_name", "my_dut", "1.0").start().await?;
    /// run.error_with_details(
    ///     &Error::builder("symptom")
    ///         .message("Error message")
    ///         .source("file", 1)
    ///         .add_software_info(&SoftwareInfo::builder("id", "name").build())
    ///         .build(),
    /// ).await?;
    /// run.end(TestStatus::Complete, TestResult::Pass).await?;
    ///
    /// # Ok::<(), WriterError>(())
    /// # });
    /// ```
    pub async fn error_with_details(
        &self,
        error: &error::Error,
    ) -> Result<(), emitter::WriterError> {
        let emitter = &self.run.state.lock().await.emitter;

        let artifact = spec::TestRunArtifact {
            artifact: spec::TestRunArtifactDescendant::Error(error.to_artifact()),
        };
        emitter
            .emit(&spec::RootArtifact::TestRunArtifact(artifact))
            .await?;

        Ok(())
    }

    pub fn step(&self, name: &str) -> TestStep {
        let step_id = format!("step_{}", self.step_seqno.fetch_add(1, Ordering::AcqRel));
        TestStep::new(&step_id, name, self.run.state.clone())
    }
}

pub struct TestRunStart {
    name: String,
    version: String,
    command_line: String,
    parameters: Map<String, Value>,
    metadata: Option<Map<String, Value>>,
    dut_info: dut::DutInfo,
}

impl TestRunStart {
    pub fn builder(
        name: &str,
        version: &str,
        command_line: &str,
        parameters: &Map<String, Value>,
        dut_info: &dut::DutInfo,
    ) -> TestRunStartBuilder {
        TestRunStartBuilder::new(name, version, command_line, parameters, dut_info)
    }

    pub fn to_artifact(&self) -> spec::RootArtifact {
        spec::RootArtifact::TestRunArtifact(spec::TestRunArtifact {
            artifact: spec::TestRunArtifactDescendant::TestRunStart(spec::TestRunStart {
                name: self.name.clone(),
                version: self.version.clone(),
                command_line: self.command_line.clone(),
                parameters: self.parameters.clone(),
                metadata: self.metadata.clone(),
                dut_info: self.dut_info.to_spec(),
            }),
        })
    }
}

pub struct TestRunStartBuilder {
    name: String,
    version: String,
    command_line: String,
    parameters: Map<String, Value>,
    metadata: Option<Map<String, Value>>,
    dut_info: dut::DutInfo,
}

impl TestRunStartBuilder {
    pub fn new(
        name: &str,
        version: &str,
        command_line: &str,
        parameters: &Map<String, Value>,
        dut_info: &dut::DutInfo,
    ) -> TestRunStartBuilder {
        TestRunStartBuilder {
            name: name.to_string(),
            version: version.to_string(),
            command_line: command_line.to_string(),
            parameters: parameters.clone(),
            metadata: None,
            dut_info: dut_info.clone(),
        }
    }

    pub fn add_metadata(mut self, key: &str, value: Value) -> TestRunStartBuilder {
        self.metadata = match self.metadata {
            Some(mut metadata) => {
                metadata.insert(key.to_string(), value.clone());
                Some(metadata)
            }
            None => {
                let mut metadata = Map::new();
                metadata.insert(key.to_string(), value.clone());
                Some(metadata)
            }
        };
        self
    }

    pub fn build(self) -> TestRunStart {
        TestRunStart {
            name: self.name,
            version: self.version,
            command_line: self.command_line,
            parameters: self.parameters,
            metadata: self.metadata,
            dut_info: self.dut_info,
        }
    }
}

pub struct TestRunEnd {
    status: spec::TestStatus,
    result: spec::TestResult,
}

impl TestRunEnd {
    pub fn builder() -> TestRunEndBuilder {
        TestRunEndBuilder::new()
    }

    pub fn to_artifact(&self) -> spec::RootArtifact {
        spec::RootArtifact::TestRunArtifact(spec::TestRunArtifact {
            artifact: spec::TestRunArtifactDescendant::TestRunEnd(spec::TestRunEnd {
                status: self.status.clone(),
                result: self.result.clone(),
            }),
        })
    }
}

#[derive(Debug)]
pub struct TestRunEndBuilder {
    status: spec::TestStatus,
    result: spec::TestResult,
}

#[allow(clippy::new_without_default)]
impl TestRunEndBuilder {
    pub fn new() -> TestRunEndBuilder {
        TestRunEndBuilder {
            status: spec::TestStatus::Complete,
            result: spec::TestResult::Pass,
        }
    }
    pub fn status(mut self, value: spec::TestStatus) -> TestRunEndBuilder {
        self.status = value;
        self
    }

    pub fn result(mut self, value: spec::TestResult) -> TestRunEndBuilder {
        self.result = value;
        self
    }

    pub fn build(self) -> TestRunEnd {
        TestRunEnd {
            status: self.status,
            result: self.result,
        }
    }
}

// TODO: this likely will go into the emitter since it's not the run's job to emit the schema version
pub struct SchemaVersion {
    major: i8,
    minor: i8,
}

#[allow(clippy::new_without_default)]
impl SchemaVersion {
    pub fn new() -> SchemaVersion {
        SchemaVersion {
            major: spec::SPEC_VERSION.0,
            minor: spec::SPEC_VERSION.1,
        }
    }

    pub fn to_artifact(&self) -> spec::RootArtifact {
        spec::RootArtifact::SchemaVersion(spec::SchemaVersion {
            major: self.major,
            minor: self.minor,
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::spec;

    #[test]
    fn test_schema_creation_from_builder() -> Result<()> {
        let version = SchemaVersion::new();
        assert_eq!(version.major, spec::SPEC_VERSION.0);
        assert_eq!(version.minor, spec::SPEC_VERSION.1);
        Ok(())
    }
}
