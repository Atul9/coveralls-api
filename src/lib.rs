extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate md5;
extern crate hyper;
extern crate hyper_native_tls;


use std::io;
use std::path::Path;
use std::fs::File;
use std::io::prelude::*;
use std::collections::HashMap;
use serde::ser::{Serialize, Serializer, SerializeStruct};
use hyper::Client;
use hyper::client::Response;
use hyper::net::HttpsConnector;
use hyper_native_tls::NativeTlsClient;


/// Representation of branch data
pub struct BranchData {
    pub line_number: usize,
    pub block_name: usize,
    pub branch_number: usize,
    pub hits: usize,
}

/// Expands the line map into the form expected by coveralls (includes uncoverable lines)
fn expand_lines(lines: &HashMap<usize, usize>, line_count: usize) -> Vec<Option<usize>> {
    (0..line_count).map(|x| match lines.get(&(x+1)){
        Some(x) => Some(*x),
        None => None
    }).collect::<Vec<Option<usize>>>()
}

/// Expands branch coverage into the less user friendly format used by coveralls -
/// an array with the contents of the structs repeated one after another in an array.
fn expand_branches(branches: &Vec<BranchData>) -> Vec<usize> {
    branches.iter()
            .flat_map(|x| vec![x.line_number, x.block_name, x.branch_number, x.hits])
            .collect::<Vec<usize>>()
}




/// Struct representing source files and the coverage for coveralls
#[derive(Serialize)]
pub struct Source {
    /// Name of the source file. Represented as path relative to root of repo
    name: String,
    /// MD5 hash of the source file
    source_digest: String,
    /// Coverage for the source. Each element is a line with the following rules:
    /// None - not relevant to coverage
    /// 0 - not covered
    /// 1+ - covered and how often
    coverage: Vec<Option<usize>>,
    /// Branch data for branch coverage.
    #[serde(skip_serializing_if="Option::is_none")]
    branches: Option<Vec<usize>>,
    /// Contents of the source file (Manual Repos on Enterprise only)
    #[serde(skip_serializing_if="Option::is_none")]
    source: Option<String>
}


impl Source {
    /// Creates a source description for a given file.
    /// display_name: Name given to the source file
    /// repo_path - Path to file relative to repository root 
    /// path - absolute path on file system
    /// lines - map of line numbers to hits
    /// branches - optional, vector of branches in code
    pub fn new(repo_path: &Path, 
           path: &Path, 
           lines: &HashMap<usize, usize>, 
           branches: &Option<Vec<BranchData>>,
           include_source: bool) -> Result<Source, io::Error> {
        
        let mut code = File::open(path)?;
        let mut content = String::new();
        code.read_to_string(&mut content)?;
        let src = if include_source {
            Some(content.clone())
        } else {
            None
        };

        let brch = match branches {
            &Some(ref b) => Some(expand_branches(&b)),
            &None => None,
        };
        let line_count = content.lines().count();
        Ok(Source {
            name: repo_path.to_str().unwrap_or("").to_string(),
            source_digest: format!("{:x}", md5::compute(content)),
            coverage:  expand_lines(lines, line_count),
            branches: brch,
            source:src,
        })
    }
}


/// Service's are used for CI integration. Coveralls current supports
/// * travis ci
/// * travis pro
/// * circleCI
/// * Semaphore
/// * JenkinsCI
/// * Codeship
pub struct Service {
    service_name: String,
    service_job_id: String,
}

/// Repo tokens are alternatives to Services and involve a secret token on coveralls
pub enum Identity {
    RepoToken(String),
    ServiceToken(Service)
}


/// Coveralls report struct 
/// for more details: https://coveralls.zendesk.com/hc/en-us/articles/201350799-API-Reference 
pub struct CoverallsReport {
    id: Identity,
    /// List of source files which includes coverage information.
    source_files: Vec<Source>,
}


impl CoverallsReport {
    /// Create new coveralls report given a unique identifier which allows 
    /// coveralls to identify the user and project
    pub fn new(id: Identity) -> CoverallsReport {
        CoverallsReport {
            id: id,
            source_files: Vec::new()
        }
    }

    /// Add generated source data to coveralls report.
    pub fn add_source(&mut self, source: Source) {
        self.source_files.push(source);
    }

    pub fn send_to_coveralls(&self) -> hyper::Result<Response> {
        self.send_to_endpoint("https://coveralls.io/api/v1/jobs")
    }

    pub fn send_to_endpoint(&self, url: &str) -> hyper::Result<Response> {
        let body = match serde_json::to_string(&self) {
            Ok(body) => body,
            Err(e) => panic!("Error {}", e),
        };      
        
        let ssl = NativeTlsClient::new().unwrap();
        let connector = HttpsConnector::new(ssl);
        let client = Client::with_connector(connector);
        client.post(url)
              .body(body.as_bytes())
              .send()
    }
}


impl Serialize for CoverallsReport {
    
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
        let size = 1 + match self.id {
            Identity::RepoToken(_) => 1,
            Identity::ServiceToken(_) => 2,
        };
        let mut s = serializer.serialize_struct("CoverallsReport", size)?;
        match self.id {
            Identity::RepoToken(ref r) => {
                s.serialize_field("repo_token", &r)?;
            },
            Identity::ServiceToken(ref serv) => {
                s.serialize_field("service_name", &serv.service_name)?;
                s.serialize_field("service_job_id", &serv.service_job_id)?;
            },
        }
        s.serialize_field("source_files", &self.source_files)?;
        s.end()
    }
}


#[cfg(test)]
mod tests {

    use std::collections::HashMap;
    use ::*;

    #[test]
    fn test_expand_lines() {
        let line_count = 10;
        let mut example: HashMap<usize, usize> = HashMap::new();
        example.insert(5, 1);
        example.insert(6, 1);
        example.insert(8, 2);
        
        let expected = vec![None, None, None, None, Some(1), Some(1), None, Some(2), None, None];

        assert_eq!(expand_lines(&example, line_count), expected);
    }

    #[test]
    fn test_branch_expand() {
        let b1 = BranchData {
            line_number: 3,
            block_name: 1,
            branch_number: 1,
            hits: 1,
        };
        let b2 = BranchData {
            line_number:4,
            block_name: 1,
            branch_number: 2,
            hits: 0
        };

        let v = vec![b1, b2];
        let actual = expand_branches(&v);
        let expected = vec![3,1,1,1,4,1,2,0];
        assert_eq!(actual, expected);    
    }

}
